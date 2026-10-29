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

Rust currently implements `components`, `evidence`, the complete
`interpretation` stage, `gaps` and `quantity`. The component
result is `{"components": [...], "blockers": [...]}` with the exact Python
materialization and integrity blockers. Its invalid authored-active rows stay
visible with `effective_state="draft"`; their anchor quarantine and readiness
barriers remain intact. Only finalized projection is explicitly skipped. Interpretation includes native component reconciliation and the final
Python compilation/precedence; the [completion checkpoint](#complete-interpretation-checkpoint-r2c)
records its proof. The [gap checkpoint](#gap-search-checkpoint-r2d) records the
bounded search and separate advisory/accounting populations. The
[quantity checkpoint](#complete-quantity-runtime-checkpoint-r2e) records complete
runtime composition and scoped tax barriers. The checkpoints below describe
historical states. Schema 1's existing raw facts suffice; no stage adds a Python
verdict to the input.

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


## Channel lifecycle and loan roles checkpoint (R2c, partial)

Branch `t3code/r2c-interpretation-10` starts at `a16fa471`. The unpackaged raw
comparison example adds `partial.interpretation.channels` and composes its
pairs, role quarantines and non-event ids into `partial.interpretation.safety`.
Python remains authoritative, snapshot v1 remains raw-only, and the product
continues to report `interpretation` under `skipped_stages`.

The Rust lifecycle port reproduces `channel_context_from_records` and
`lightning/channel_lifecycle.py`: canonical funding txid/outpoint agreement,
independent wallet/backend/adapter scope, BTC-only transaction scope, strict
`lnd:`/`coreln:` atomicity, batched balances, ambiguous node ownership and
funding-before-close eligibility. Direct close payouts take precedence over
vin matches. A force-close sweep requires the curated LND adapter's exact
sweep-txid/local-commitment-outpoint marker; unique, competing, duplicated and
later peer-output payments preserve Python's selection and mismatch behavior.
One balance-minus-group-receipts fee rides on the greatest `(amount, id)` leg.
Both directions of the balance gap use Python's float percentage/floor ceiling,
including exact boundary and overflow behavior. Cloned rows retain their raw
anchor, node account refs, original amounts and pricing fields.

Python has no distinct splice interpreter. Any stored tag other than
`channel_close` follows the funding branch, including the splice-tag control.
This port reproduces that behavior without granting new splice authority.

Channel roles are computed before public pair construction. Within the
interpreter, Samourai preparation and gated recorded matching retain their
order; channel pairs follow consolidation, ownership and fan-out candidates.
Role mismatches enter holds as existing blockers, and their quarantines follow
pair quarantines and precede privacy quarantines. Every channel role, channel
pair anchor and truthy loan id enters the occupied/resolved population. Holds,
privacy, native audits and authored exclusions retain their existing authority.
Pending channel/loan observations still block; a non-event id never clears a
quarantine or proves a custody target.

All four persisted loan roles contribute non-event ids exactly as Python does:
`collateral_lock` and `loan_principal_repaid` are authored for outbound rows;
`collateral_release` and `loan_principal_received` for inbound rows. Python's
mark-authoring API enforces those directions with `validation` errors. The
interpretation stage does not revalidate persisted role/direction or invent a
virtual lender/location. Explicit wrong-direction, unknown-role and empty-id
controls pin that distinction, rather than silently changing legacy marks.
The storage/authoring API itself is outside this pure R2 port.

Integer conversion reuses the existing Python-compatible stored scalar parser,
including its digit limit and error families; no raw `codec::wide()` call is
added. Curated JSON keeps nonfinite and lone-surrogate scalars until admission.
Unhashable curated `import_source` values retain Python's set-membership
`TypeError`, including role/pair propagation and admission order.
Invalid surrogate identities decline without panicking. Existing corrected
microsecond/offset timestamp parsing remains in the pair/hold helpers; lifecycle
leg ordering itself uses Python's literal timestamp-string/id order.

### Branch witnesses and equivalences

The frozen partial corpus has **288 raw snapshots** (254 retained and 34 new),
including role-only reservation and malformed curated-provenance witnesses.
The **404 direct helper vectors** live in
`tests/fixtures/custody_channel_helpers.json`; they never add derived operands
to raw snapshot `input`. Their generated Python outputs pin scope, outpoints,
vin wrappers, curated sweep provenance, fee boundaries, unbounded arithmetic,
invalid integer/column errors, duplicate owners, role atomicity and construction
order. The Rust fixture reader caches its immutable parsed corpus so repeated
named witnesses reuse it.

| Mutation group | Entries | Named killing tests |
| --- | ---: | --- |
| Retained catalog | 795 | Retained evidence, component, producer, graph and safety witnesses |
| Lifecycle helpers and production | 86 | `channel_lifecycle_helper_vectors` |
| Channel/loan composition | 8 | `interpretation_channel_projection` |
| Total | **889** | **All killed** |

Every ported Python branch has a catalog witness or the following explicit
disposition:

- `_field`/`_row_dict` object and sqlite-row adapters are represented by normalized
  raw record mappings at snapshot v1; arbitrary Python objects do not cross it.
  Required journal record columns retain `KeyError` and read order. The typed
  raw mapping functions preserve explicit nulls rather than inventing columns.
  Empty channel records return before transaction amount admission, with a
  separate malformed-amount witness and killed mutation.
- Empty/nonmapping `_raw_mapping` results are equivalent: later `get`, list and
  mapping admission yields no scope/outpoint/sweep. Skipping a false value before
  JSON parsing is equivalent to parsing it and returning an empty mapping.
  Nonempty decoded mappings, stored strings, nested tx mappings, invalid JSON,
  nonfinite metadata and surrogate identities have separate witnesses.
- `_canonical_outpoint` colon presence, parse errors and nonnegative bounds are
  witnessed. Its extra outer whitespace strip is retained; C0
  whitespace differs from the integer component parser and has a killed witness.
  The final typed-txid canonicalizer is the already mutated shared predicate.
  Surrogate scalar identities cannot be canonical hex or integer indices, so
  declining them before UTF-8 conversion is equivalent to Python's rejection.
- BTC asset admission plus the shared physical-scope contract implies Bitcoin
  chain scope; independently removing only the final chain equality is
  equivalent. The independent asset guard is mutation-killed.
- Iterating no closing scopes/eligible scopes, no candidates, or no vin matches
  emits no close legs. Removing only those empty iteration shortcuts is
  equivalent. The separate guard that skips pair construction when there are
  no atomic channel rows preserves amount admission and has its own malformed
  amount witness and killed mutation. Direct-vs-vin precedence, one-vs-many
  candidates, attestation selection, duplicate outputs, chronological leg order
  and first-seen scope order have distinct killed mutations.
- Positive batched contributions share `add_positive`; its accumulation and
  negative-value admission are mutated. Including an extra zero in a sum is
  equivalent. `_funding_amount_mismatch` and `_close_balance_mismatch` also
  retain their directly callable negative branches, even though aggregate
  balances include only positive records.
- `_remember_owner`'s ambiguous set is sticky. Strict zero metadata and later
  repeated owners are witnessed. The eligible close map's explicit ambiguity
  exclusion is redundant because `remember` never restores a removed owner.
  Similarly, no closing owner means the eligible set is empty. Both guards
  remain represented without claiming equivalent mutations as kills.
- Linkage can be direct on a combined channel row or recovered from separate
  records by `(wallet_id, channel_id)` within one network. The stored loader
  emits separate funding/closing rows; combined rows remain direct-helper
  witnesses. Truthy linked-funding existence is equivalent to inserting an
  empty set when no same-network funding remains.
- Every close group has selected legs: a skipped unique peer match requires an
  existing direct payout. Therefore the fee keeper's nonempty-leg check and
  `pair.kind == CHANNEL_CLOSE` after an emitted non-open atomic pair are
  exhaustive typed-producer facts. Zero-fee keeper selection cannot alter any
  row; fee amount, single-leg attribution and ties are separately mutated.
- Generic covered-id calculation already includes every channel pair anchor;
  adding those anchors again to occupied/resolved ids is equivalent for holds.
  Role-only and imported-loan reservations have independent mutation kills.
  Repeating an identical loan mark is set-equivalent. Missing/null/empty loan
  ids, false-but-present ids, all four valid roles and legacy invalid marks are
  pinned without introducing a direction check absent from interpretation.
- Native HTLC production/reservations and native component reconciliation are
  still unported. Their empty input/proof branches are not claimed here.

### Scenario counts

Each row pins **both Python and Rust** independently. `Books` is the number
compared per engine; the remaining columns are exact results **per book** in
each engine: channel roles/pairs, compiled claims/audits, non-event ids and
blocked/transient rows. Pair/audit counts are pre-arbitration candidates and
never ownership or report authority.

| Scenario | Books Python / Rust | Roles | Channel pairs | Claims | Audits | Non-events | Blocked | Transient |
| --- | ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: |
| `open_close` | 63 / 63 | 2 | 2 | 0 | 2 | 2 | 0 | 0 |
| `strict_lnd` | 63 / 63 | 2 | 2 | 0 | 2 | 2 | 0 | 0 |
| `strict_coreln` | 63 / 63 | 2 | 2 | 0 | 2 | 2 | 0 | 0 |
| `batched` | 63 / 63 | 2 | 2 | 0 | 2 | 2 | 0 | 0 |
| `force_trusted` | 63 / 63 | 3 | 3 | 0 | 3 | 3 | 0 | 0 |
| `force_untrusted` | 63 / 63 | 3 | 1 | 0 | 1 | 1 | 2 | 0 |
| `force_multiple` | 63 / 63 | 4 | 4 | 0 | 4 | 4 | 0 | 0 |
| `force_duplicate` | 63 / 63 | 4 | 1 | 0 | 1 | 1 | 3 | 0 |
| `peer_payment` | 63 / 63 | 2 | 2 | 0 | 2 | 2 | 0 | 0 |
| `fee_tie` | 63 / 63 | 3 | 3 | 0 | 3 | 3 | 0 | 0 |
| `missing_funding` | 63 / 63 | 1 | 0 | 0 | 0 | 1 | 0 | 0 |
| `open_mismatch` | 63 / 63 | 2 | 0 | 0 | 0 | 1 | 1 | 0 |
| `close_mismatch` | 63 / 63 | 2 | 1 | 0 | 1 | 1 | 1 | 0 |
| `strict_zero` | 63 / 63 | 2 | 0 | 0 | 0 | 0 | 2 | 0 |
| `owner_conflict` | 63 / 63 | 2 | 0 | 0 | 0 | 1 | 1 | 0 |
| `network_conflict` | 63 / 63 | 1 | 1 | 0 | 1 | 1 | 0 | 0 |
| `graph_conflict` | 62 / 62 | 1 | 1 | 0 | 1 | 1 | 0 | 0 |
| `pending_open` | 62 / 62 | 2 | 2 | 0 | 1 | 2 | 1 | 1 |
| `privacy_close` | 62 / 62 | 2 | 2 | 0 | 2 | 2 | 1 | 0 |
| `splice_tag` | 62 / 62 | 2 | 2 | 0 | 2 | 2 | 0 | 0 |
| `loan_collateral` | 62 / 62 | 0 | 0 | 0 | 0 | 2 | 0 | 0 |
| `loan_principal` | 62 / 62 | 0 | 0 | 0 | 0 | 2 | 0 | 0 |
| `loan_all` | 62 / 62 | 0 | 0 | 0 | 0 | 4 | 0 | 0 |
| `loan_pending` | 62 / 62 | 0 | 0 | 0 | 0 | 1 | 1 | 1 |
| `mixed_recorded` | 62 / 62 | 2 | 2 | 1 | 2 | 2 | 0 | 0 |
| `mixed_pipeline` | 62 / 62 | 2 | 2 | 4 | 3 | 2 | 0 | 0 |
| `mixed_samourai` | 62 / 62 | 2 | 2 | 1 | 2 | 2 | 0 | 0 |
| `loan_wrong_direction` | 62 / 62 | 0 | 0 | 0 | 0 | 2 | 0 | 0 |
| `loan_unknown_role` | 62 / 62 | 0 | 0 | 0 | 0 | 1 | 0 | 0 |
| `loan_empty_ids` | 62 / 62 | 0 | 0 | 0 | 0 | 3 | 0 | 0 |
| `strict_owner_conflict` | 62 / 62 | 2 | 0 | 0 | 0 | 0 | 2 | 0 |
| `multiple_close_scopes` | 62 / 62 | 4 | 4 | 0 | 4 | 4 | 0 | 0 |

The generator compares **2,000 raw books**, **64 permutations**, and **64 full
Python interpreter controls**, including mixed recorded, Samourai and composed
graph producers. Independent assertions also conserve the selected close group's
fee, charging exactly its greatest `(amount, id)` leg. Assertions pin every count
above for each engine separately, then compare complete outputs and quarantine
details. Per engine, the new corpus totals **3,509 channel roles, 2,566 channel
pairs, 372 claims, 2,566 rowless audits, 3,685 non-event ids, 940 blocked rows and
124 transient rows**. The retained generators keep their own assertion matrices.
Full census and custody-shadow shards remain the orchestrator's verification
lanes.

### Verification and remaining work

Verification uses `mise exec rust@1.99.1 uv@latest -- ...`,
`MATURIN_NO_INSTALL_RUST=1`, a locked Python 3.11 environment, offline Cargo
(including `CARGO_NET_OFFLINE=true` in child builds), disposable data/cache roots
and local synthetic fixtures. No dependency, schema field, product Python
behavior, persistent logging or external I/O changes.

- `cargo fmt --all --check` and workspace/all-target Clippy with warnings
  denied pass. Offline, locked workspace tests excluding `kassiber-net` pass
  **728 tests**. Ledger JSON feature comparison retains **145 byte-identical
  outputs**.
- Inventory validation and inventory/contracts/mutation-checker contracts pass
  **13 tests and 14 subtests**. Whole-catalog applicability reports **889 entries
  apply**, naming **311 distinct test filters**.
- The standalone channel/loan suite passes **three tests**, including 2,000
  generated books with independent per-engine scenario counts, 64 permutations
  and 64 full-Python controls. Frozen/reference regeneration and the authoring
  direction controls separately pass **three tests**. The final 2,000-book
  fee-conservation rerun also passes.
- A fresh focused census/recorder run passes **107 tests** and records **62 raw
  snapshots**. Fresh-process integrity replay passes. Rust replay compares **62
  requests and 62 permutations**, with zero pre-request refusals and zero
  differences; the replay also passes after the final manual review.
- All **889 mutations are killed**: 795 retained entries in four disjoint runs
  of 199/199/199/198, 93 new entries in three disjoint runs of 31/31/31, and the
  final empty-atomic-row admission witness in its own run.
  The combined kill ids equal the catalog exactly, with no duplicate or missing
  entry. Every run proves its named baselines before mutation; no survivor or
  nonbuilding mutation is counted as a kill.

- The broad custody, channel lifecycle, loan, ownership, transfer, review and
  onchain modules report **1,094 passed and 2,700 subtests passed, with nine
  failures**. All nine generated suites complete, comparing **18,000 seeded
  books**. Five daemon tests require the unchanged stable owner-lock runtime
  `~/.kassiber/run`, which is read-only in this sandbox: the two
  `CustodyComponentDaemonSurfaceTests`, the two real-JSONL
  `CustodyGapDaemonProtocolAcceptanceTests`, and
  `test_cold_daemon_opens_plaintext_book_before_handoff_scope_check`.
  Returned error-envelope diagnostics confirm the runtime restriction behind
  their missing-`data` failures. Four `ReviewRegressionTest` BTCPay sync cases
  fail creating local HTTP test sockets with `EPERM`. These match the previous
  safety handoff's sandbox limits; no runtime guard is weakened or real book
  used. This run does not claim those nine tests passed.

Remaining R2 slices: native HTLC transition production and reservations, native
component reconciliation, and final interpreter composition. Keep interpretation
skipped until that entire stage compares. Gap search, quantity arbitration and
finalized projection follow before custody joins the ledger. No push or PR is
made at this checkpoint.

## Native HTLC production and reservations checkpoint (R2c partial)

Branch `t3code/r2c-interpretation-11` starts at `a4e017a3`. The test-only
`partial.interpretation.native` projection now ports the native producer;
`partial.interpretation.safety` composes its pairs, proofs, quarantines and
reservations with the existing producers. Python remains authoritative.
Snapshot v1 stays raw-only, and production keeps `interpretation` in
`skipped_stages` until reconciliation and the complete stage compare.

The producer matches the original hash/refund/provider population before
applying dismissals or occupied anchors. A dismissed or reviewed sibling cannot
promote an ambiguous route into an exact one. Independent provider conflicts
remain in the global candidate graph. Ordered edge deduplication and union-find
preserve Python's minimum conflict key and edge cardinality without repeatedly
expanding the complete graph. Ambiguity holds use the reference's sorted amount
windows rather than materializing duplicate hash populations' Cartesian product.

Current LWK/Core attestations bind observer kind, graph and quantity hashes,
role, single-input topology and funding outpoint. Legacy public witness replay
checks the Boltz script template and hashlock before recovering a claim hash or
timeout funding outpoint. Metadata precedence, whole funding-output coverage,
known network domains, exact canonical principals, policy and UTC fee dates
remain separate admission checks. A cross-date shortfall stays quarantined;
a zero-shortfall route does not acquire that fee-date barrier.

LND and CLN settlement and aliases compare. NWC is a negative authority control:
Python's native payment-hash predicate does not admit NWC. Failed attempts
contribute no native transition; failed swaps with a proven timeout refund use
the refund path. A legacy failed-attempt row is a negative control, not a new
adapter ingestion rule. No node adapter, acquisition, network,
logging, storage or disclosure behavior changes. Synthetic public script
fixtures exercise legacy witness compatibility; no node preimage, payment secret,
route or credential field is added to snapshots or product surfaces.

Composition follows Python's order: authored exclusions, Samourai touches,
complete recorded pairs, channel roles/anchors and persisted loan marks occupy
anchors before native production. Native proof anchors and quarantines reserve
graph derivation; proven native anchors also resolve privacy holds. Native pairs
then join the existing candidate order before raw holds, proof-aware pair
compilation and dependency closure. Test-only `native_transitions` and
`handled_ids` outputs independently witness these reservation boundaries.
Native component reconciliation is intentionally absent from this checkpoint.

### Branch witnesses and equivalences

The frozen partial corpus has **343 raw snapshots**, including 55 new native
scenarios. The **1,078 direct helper vectors** in
`tests/fixtures/custody_native_helpers.json` are separate from snapshot `input`.
They pin malformed attestations, witness bytes, script truncation, funding values,
provider aliases and contradictions, Unicode integer admission, nonfinite raw
metadata, timestamp errors, policy, full-population conflicts and proof binding.
The 59-case native projection test includes four existing reservation witnesses.

| Mutation group | Entries | Named killing tests |
| --- | ---: | --- |
| Retained catalog | 889 | Existing evidence, component, producer, graph and safety tests |
| HTLC proof helpers | 39 | `native_htlc_helper_vectors` |
| Matcher and independent provider conflicts | 57 | `native_matcher_helper_vectors` |
| Native production | 38 | `native_production_helper_vectors`, `interpretation_native_projection` |
| Reservation/composition | 12 | `interpretation_native_projection` |
| Total | **1,035** | **All killed** |

Every ported Python branch is mutation-witnessed or has this explicit disposition:

- Native compilation fixes `include_heuristics=False`, with empty matcher pair
  records, dismissals and booked-MOVE inputs. Their heuristic, existing-pair and
  matcher-level veto branches are unreachable here; native dismissals remain a
  separately witnessed producer admission. Removing empty iteration shortcuts,
  path compression, or repeated identical set inserts is equivalent.
- Snapshot records normalize sqlite rows/mappings. Arbitrary Python objects,
  raw bytes/tuples and unsupported mapping implementations cannot cross v1.
  Transaction amounts/fees are signed-64 integers at snapshot admission; direct
  helper vectors additionally preserve Python's unbounded conversions and
  uncaught fee `OverflowError`. Helper-only infinity decoding never changes v1.
- CPython case/strip and integer helpers are reused. Provider `isdigit` preserves
  CPython 3.11 Unicode 14's distinction from decimal digits: digit-only strings
  that `int` rejects still raise `ValueError`. Private surrogate identities use
  lossless internal encoding; they never escape the candidate topology module.
- The corrected microsecond/offset time parser is reused. The explicit matcher
  clock is parsed before row admission, as in Python. Caught `TypeError` and
  `ValueError` remain distinct from propagated `OverflowError`; an unhashable
  observer retains `TypeError`. The reference script parser's indexed equality
  read on a recognized 22-byte truncation retains `IndexError`.
- The native proof consumes candidate topology, ordering, confidence, method,
  default kind and conflict fields. Display labels, evidence descriptions and
  unused default policy are omitted from the comparison projection; all fields
  used in pair production, proof validation and candidate ordering compare.
  Raw asset/quantity/authority fields remain available on the original rows.
- Presence checks preserve the string marker shortcut, including escaped-marker
  behavior, and mapping/nested-vin alternatives. Empty/nonmapping decoded raw
  payloads produce no attestation, witness or funding value. Empty loops over
  those shapes are equivalent to their explicit short-circuits.
- Script/witness guards, claim-vs-timeout classification, metadata-vs-recovery
  precedence, one-vs-many inputs, duplicate funding candidates and exact whole
  output coverage are independently witnessed. HASH160 requires RIPEMD160,
  absent from std and the existing SHA-256 dependency; `bitcoin_hashes` adds only
  pure hashing, with the lockfile and license inventory updated.
- Provider display-version/taproot/cooperative/spend-path fields do not affect
  the consumed projection or any native proof. Missing/false metadata fallbacks
  are equivalent when they select the same absent evidence. Provider identity,
  semantic, route and amount conflicts and the independently competing edge
  branches have killed mutations.
- Native anchors already occur in native pair inputs when holds calculates
  covered ids, so removing only their duplicate resolved-id insert is equivalent.
  Channel pair anchors already occur in typed channel roles for occupancy;
  removing only that duplicate occupancy insert is equivalent. Role-only,
  Samourai, recorded, loan, derivation and privacy reservations have independent
  mutation witnesses. These equivalent mutations are excluded from the catalog.

### Scenario counts

Each row is asserted independently for **Python and Rust**. Books are compared
per engine; all other columns are exact counts **per book in both engines**.
Candidates and claims remain pre-arbitration evidence, not ownership or report
readiness. Standalone native counts precede the composed occupancy checks, so
an occupied loan/channel can still have a standalone native pair while the
composed interpreter reserves it.

| Scenario | Books Python / Rust | Candidates | Native pairs | Proofs | Native quarantines | Claims | Audits | Non-events | Blocked | Transient |
| --- | ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: |
| `lnd_settle` | 37 / 37 | 1 | 1 | 1 | 0 | 2 | 0 | 0 | 0 | 0 |
| `cln_settle` | 37 / 37 | 1 | 1 | 1 | 0 | 2 | 0 | 0 | 0 | 0 |
| `cln_alias_settle` | 37 / 37 | 1 | 1 | 1 | 0 | 2 | 0 | 0 | 0 | 0 |
| `nwc_untrusted` | 37 / 37 | 0 | 0 | 0 | 0 | 0 | 0 | 0 | 0 | 0 |
| `lnd_forged` | 37 / 37 | 0 | 0 | 0 | 0 | 0 | 0 | 0 | 0 | 0 |
| `reverse_settle` | 37 / 37 | 1 | 1 | 1 | 0 | 2 | 0 | 0 | 0 | 0 |
| `timeout_refund` | 37 / 37 | 1 | 1 | 1 | 0 | 2 | 0 | 0 | 0 | 0 |
| `raw_refund` | 37 / 37 | 1 | 1 | 1 | 0 | 2 | 0 | 0 | 0 | 0 |
| `raw_claim` | 37 / 37 | 1 | 1 | 1 | 0 | 2 | 0 | 0 | 0 | 0 |
| `core_attestation` | 37 / 37 | 1 | 1 | 1 | 0 | 2 | 0 | 0 | 0 | 0 |
| `fee_next_day` | 37 / 37 | 1 | 0 | 0 | 2 | 0 | 0 | 0 | 2 | 0 |
| `fee_unknown_time` | 37 / 37 | 1 | 0 | 0 | 2 | 0 | 0 | 0 | 2 | 0 |
| `zero_fee_next_day` | 37 / 37 | 1 | 1 | 1 | 0 | 1 | 0 | 0 | 0 | 0 |
| `utc_same_day` | 37 / 37 | 1 | 1 | 1 | 0 | 2 | 0 | 0 | 0 | 0 |
| `utc_different_day` | 37 / 37 | 1 | 0 | 0 | 2 | 0 | 0 | 0 | 2 | 0 |
| `dismiss_active` | 37 / 37 | 1 | 0 | 0 | 0 | 0 | 0 | 0 | 0 | 0 |
| `dismiss_expired` | 37 / 37 | 1 | 1 | 1 | 0 | 2 | 0 | 0 | 0 | 0 |
| `dismiss_equal` | 37 / 37 | 1 | 1 | 1 | 0 | 2 | 0 | 0 | 0 | 0 |
| `dismiss_invalid` | 37 / 37 | 1 | 0 | 0 | 0 | 0 | 0 | 0 | 0 | 0 |
| `dismiss_future` | 37 / 37 | 1 | 0 | 0 | 0 | 0 | 0 | 0 | 0 | 0 |
| `duplicate_ln` | 36 / 36 | 0 | 0 | 0 | 3 | 0 | 0 | 0 | 3 | 0 |
| `duplicate_chain` | 36 / 36 | 0 | 0 | 0 | 3 | 0 | 0 | 0 | 3 | 0 |
| `duplicate_occupied` | 36 / 36 | 0 | 0 | 0 | 3 | 0 | 0 | 1 | 2 | 0 |
| `duplicate_dismissed` | 36 / 36 | 0 | 0 | 0 | 2 | 0 | 0 | 0 | 2 | 0 |
| `network_mismatch` | 36 / 36 | 0 | 0 | 0 | 0 | 0 | 0 | 0 | 0 | 0 |
| `network_unknown` | 36 / 36 | 0 | 0 | 0 | 0 | 0 | 0 | 0 | 0 | 0 |
| `amount_increase` | 36 / 36 | 0 | 0 | 0 | 0 | 0 | 0 | 0 | 0 | 0 |
| `amount_excess_fee` | 36 / 36 | 0 | 0 | 0 | 0 | 0 | 0 | 0 | 0 | 0 |
| `canonical_mismatch` | 36 / 36 | 1 | 0 | 0 | 2 | 0 | 0 | 0 | 2 | 0 |
| `source_untrusted` | 36 / 36 | 1 | 0 | 0 | 0 | 0 | 0 | 0 | 0 | 0 |
| `target_stale` | 36 / 36 | 0 | 0 | 0 | 0 | 0 | 0 | 0 | 0 | 0 |
| `observer_mismatch` | 36 / 36 | 0 | 0 | 0 | 0 | 0 | 0 | 0 | 0 | 0 |
| `occupied_loan` | 36 / 36 | 1 | 1 | 1 | 0 | 0 | 0 | 1 | 0 | 0 |
| `occupied_channel` | 36 / 36 | 1 | 1 | 1 | 0 | 0 | 1 | 1 | 0 | 0 |
| `privacy_native` | 36 / 36 | 1 | 1 | 1 | 0 | 2 | 0 | 0 | 0 | 0 |
| `pending_claim` | 36 / 36 | 1 | 1 | 1 | 0 | 0 | 0 | 0 | 2 | 2 |
| `failed_payment` | 36 / 36 | 0 | 0 | 0 | 0 | 0 | 0 | 0 | 0 | 0 |
| `timeout_missing_proof` | 36 / 36 | 1 | 0 | 0 | 0 | 0 | 0 | 0 | 0 | 0 |
| `refund_wrong_outpoint` | 36 / 36 | 1 | 0 | 0 | 0 | 0 | 0 | 0 | 0 | 0 |
| `refund_duplicate_funding` | 36 / 36 | 0 | 0 | 0 | 0 | 0 | 0 | 0 | 0 | 0 |
| `refund_partial_output` | 36 / 36 | 0 | 0 | 0 | 0 | 0 | 0 | 0 | 0 | 0 |
| `refund_batched` | 36 / 36 | 1 | 0 | 0 | 0 | 0 | 0 | 0 | 0 | 0 |
| `provider_conflict` | 36 / 36 | 2 | 0 | 0 | 2 | 0 | 0 | 0 | 2 | 0 |
| `provider_same_edge` | 36 / 36 | 1 | 1 | 1 | 0 | 2 | 0 | 0 | 0 | 0 |
| `provider_only` | 36 / 36 | 1 | 0 | 0 | 0 | 0 | 0 | 0 | 0 | 0 |
| `provider_different_network` | 36 / 36 | 1 | 1 | 1 | 0 | 2 | 0 | 0 | 0 | 0 |
| `policy_taxable` | 36 / 36 | 1 | 1 | 1 | 0 | 2 | 0 | 0 | 0 | 0 |
| `policy_austria` | 36 / 36 | 1 | 1 | 1 | 0 | 2 | 0 | 0 | 0 | 0 |
| `alias_asset` | 36 / 36 | 1 | 1 | 1 | 0 | 0 | 0 | 0 | 0 | 0 |
| `raw_reverse_prefix` | 36 / 36 | 1 | 1 | 1 | 0 | 2 | 0 | 0 | 0 | 0 |
| `raw_wrong_hash` | 36 / 36 | 0 | 0 | 0 | 0 | 0 | 0 | 0 | 0 | 0 |
| `mixed_recorded` | 36 / 36 | 1 | 1 | 1 | 0 | 3 | 0 | 0 | 0 | 0 |
| `mixed_pipeline` | 36 / 36 | 1 | 1 | 1 | 0 | 6 | 1 | 0 | 0 | 0 |
| `mixed_samourai` | 36 / 36 | 1 | 1 | 1 | 0 | 3 | 0 | 0 | 0 | 0 |
| `mixed_channel` | 36 / 36 | 1 | 1 | 1 | 0 | 2 | 2 | 2 | 0 | 0 |

The generator compares **2,000 books per engine**, **110 permutations** and
**110 full-Python interpreter controls**. Per engine, totals are **1,458
candidates, 948 native pairs, 948 proofs, 762 native quarantines, 1,787 composed
claims, 144 audits, 180 non-events, 798 blocked ids and 72 transient ids**.
Existing producers retain their independent scenario matrices.

### Verification and remaining work

Verification uses `mise exec rust@1.99.1 uv@latest -- ...`, a locked Python
3.11 environment created with `uv sync --locked --python 3.11`,
`MATURIN_NO_INSTALL_RUST=1`, offline Cargo (including child builds) and disposable
roots. Formatting, Clippy and the workspace suite also pass after the final
helper-witness additions.

- `cargo fmt --all --check` and workspace/all-target Clippy with warnings denied
  pass. Offline locked workspace tests excluding `kassiber-net` pass **732
  tests**. The selected native/helper Rust run passes **13 tests**, including
  the 59-snapshot projection. The final signed-fee ordering witness also passes
  its named unmutated baseline before killing the ordering mutation.
- Inventory classification and contracts checks pass; inventory/contracts/shared
  mutation-runner contracts pass **13 tests and 14 subtests**. Catalog
  applicability reports **1,035 entries apply**, naming **315 test filters**.
  The ledger feature comparison retains **145 byte-identical outputs**.
- Focused census and recording pass **69 tests and 21 subtests**, comparing
  every partial projection for **10 raw requests**. Fresh-process integrity
  replay passes all **10 snapshots**. Rust replay compares **10 requests and
  10 permutations**, with **zero pre-request refusals and zero differences**.
- All **1,035 mutations are killed**: **889 retained** and **146 new**. Four
  disjoint partitions account for 256/255/255/254 kills, 14 added branch
  mutations are killed separately, and one signed-fee-order mutation is killed
  after adding an independent multi-candidate ordering witness. Its earlier
  single-candidate witness survived and is not counted as a kill. The final
  kill-id union equals the catalog exactly, without duplicates, missing ids,
  nonbuilding mutations or surviving entries. Every run proves its named
  baseline before applying a mutation.
- The broad custody, lifecycle, loan, ownership and transfer modules report
  **787 passed and 2,674 subtests passed, with four failures**. All ten seeded
  2,000-book corpora complete, including the new native suite and its independent
  per-engine counts, 110 permutations and 110 full-Python interpreter controls.
  The four failures are the two `CustodyComponentDaemonSurfaceTests` and the two
  real-JSONL `CustodyGapDaemonProtocolAcceptanceTests`: their unchanged stable
  owner-lock namespace is `~/.kassiber/run`, which is read-only in this sandbox.
  A focused diagnostic rerun confirms `internal_error` / `Read-only file system`
  for that namespace in all four error envelopes. No product guard is weakened
  and none of these four failures is reported as passing.

At this historical checkpoint native component reconciliation and final
composition remained. The following checkpoint completes them; gaps, quantity
and finalized projection still precede journal fusion.


## Complete interpretation checkpoint (R2c)

Branch `t3code/r2c-interpretation-12` starts at `cc18b710`. Native component
reconciliation, the reviewed quantity compiler it consumes, exact FIFO route
allocation and physical-before-wallet chronology now match Python. The final
composition preserves producer/hold precedence, temporarily clears authored
reservations only while examining native candidates, and releases only fully
reconciled components' anchors before final claim compilation. Complete
certificates, native audits, quarantines, blocked/transient ids and non-events
are compared, including native zero-delta and legacy-migration wrappers.

The PyO3 response now lists `interpretation` in `stages`, after `components` and
`evidence`; only `gaps`, `quantity` and `finalized_projection` remain skipped.
Census and custody-shadow compare the entire stage. The unpackaged partial
executable and its automatic comparison path are removed; frozen slice vectors
remain Rust tests. Python remains authoritative, Rust outputs never reach stored
state, and snapshot v1 remains raw-only. No storage, dependency, network or
consent boundary changes.

### Branch coverage and restricted compiler scope

The new mutation entries supplement all 1,035 retained entries. Every entry
names an unmutated passing test and is killed by a failing test after a successful
build. The final catalog has **1,178 entries** and **322 named test filters**.

| Area | New entries | Named witnesses |
| --- | ---: | --- |
| Native route admission, coverage, pools and certificates | 55 | `reconciliation_helper_vectors`, `reconciliation_compilation_vectors` |
| Reviewed quantities and transactionless route flattening | 39 | `reconciliation_reviewed_vectors` |
| Closed chronology and stable source deduplication | 12 | `reconciliation_chronology_vectors`, `reconciliation_helper_vectors` |
| Exact FIFO allocation | 6 | `reconciliation_fifo_vectors` |
| Candidate reservations and selective anchor release | 3 | `reconciliation_compilation_vectors` |
| Migration JSON, iteration, scope and quarantines | 18 | `interpretation_wrapper_vectors` |
| Complete compilation and wrapper composition | 5 | `interpretation_wrapper_vectors`, `reconciliation_compilation_vectors` |
| Implemented inventory and admission boundary | 5 | `interpretation_operation_vectors` |
| **New / retained / total** | **143 / 1,035 / 1,178** | **All killed** |

The following dispositions describe intentionally restricted or equivalent
branches rather than counting artificial mutations as kills:

- Reconciliation admits only authored-active, quantity-conserving, term-free
  components. Conversion and economic-term claims are outside this private
  compiler's call boundary. They retain ordinary authored reservations; their
  admission guards have negative witnesses. Complete component claim compilation
  for quantity arbitration remains with the future quantity stage.
- The private reviewed compiler returns only claim fields consumed by
  reconciliation. Metadata, priorities and destination presentation omitted
  here cannot influence its result. Its public final output is the native
  certificate plus ordinary interpreter claims, compared in full.
- Internal FIFO buckets have unique string identities and nonnegative BigInt
  capacities constructed from admitted canonical slices, exact fees and ordered
  origin credits. Arbitrary Python bucket shape/id/type/duplicate admission is
  unreachable at this seam. Insufficient capacity and empty/zero capacities have
  direct witnesses; no raw quantity is narrowed through `codec::wide()`.
- Compiler failure families caught by Python reject the candidate component.
  Materialized ids/revisions and canonical observation references make missing
  map entries and invalid certificate revisions unreachable; test helpers retain
  malformed amount/role/direction/coverage witnesses. All reachable compiler
  branches use Python integer families and CPython case helpers.
- Chronology reuses the closed-observer gate, strict transaction ids, CPython
  scope case rules and corrected timestamp parsing already ported. Exact integer
  vout admission excludes bool/float/text, including zero floats. Stable topology,
  hard-before-soft dependencies and cycle fallback have separate witnesses.
  Repeated native claim ids are unavailable from the canonical compiler; dict
  update position is nevertheless retained. Source deduplication is pinned by
  an independent fan-out witness.
- Activity exclusion scans the same pool/time interval as Python's sorted
  bisect window. It is an order-independent `any` test, so a scan is equivalent.
  Native-source groups retain Python's raw occurred-at ordering and same-time
  chronology. Claim/source/target ordering, fee positions and intermediate
  accumulation have independent witnesses.
- Removing the transactionless stranded-pool guard still reaches the same caught
  incomplete-source compile failure at this restricted reconciliation seam.
  Changing digest `ensure_ascii` cannot affect hex quantity hashes and the
  literal `fee`. These equivalent trial mutations were removed, without being
  counted as kills.
- Decoded migration values remain CPython JSON values, preserving surrogate
  strings, nonfinite detail scalars, object insertion order and character
  iteration. Decoded keys are always strings and JSON-loaded details are always
  serializable with Python's default `ensure_ascii`; their impossible key/encoding
  errors are not synthetic mutation entries. Existing integer/string helper
  semantics remain covered by their retained mutations.
- Operation admission remains accounting/rates, channel facts, component
  materialization, migration admission, evidence, interpretation. A malformed
  source amount is refused by raw snapshot validation before these stages; it is
  not a valid witness for moving migration past evidence. Boundary vectors pin
  channel-before-component, component-before-migration and tax-gate precedence.

### Complete-stage corpus and per-scenario counts

All eleven generators compare **2,000 books each for each engine** and compare
full output over permutations of unordered inputs. Python and Rust independently
count each scenario and output signature. Exact checked-in per-scenario counts
are in [`custody_interpretation_counts.json`](../../tests/fixtures/custody_interpretation_counts.json),
asserted by `test_every_generator_matches_and_counts_both_engines`. Signature
columns are claims, native audits, reconciliations, non-event ids, blocked ids,
transient ids; equality checks compare every field, not just these counts.

| Generator | Scenarios | Books per scenario per engine | Permutations |
| --- | ---: | --- | ---: |
| channels | 32 | 62–63 | 64 |
| components | 156 | 12–13 | 312 |
| evidence | 29 | 68–69 | 58 |
| fanout | 47 | 42–43 | 94 |
| graph | 8 | 250 | 16 |
| native | 55 | 36–37 | 110 |
| pipeline | 12 | 166–167 | 24 |
| reconciliation | 25 | 80 | 50 |
| recorded | 46 | 43–44 | 92 |
| safety | 39 | 51–52 | 78 |
| samourai | 34 | 58–59 | 68 |

Total: **22,000 books and 966 permutations**, exact compilation equality.
The reconciliation generator has 25 scenarios, each with **80 books per engine**:

| Scenario | Claims per book | Certificates per book | Blocked / transient per book |
| --- | ---: | ---: | --- |
| blocked_middle | 0 | 0 | 1 / 1 |
| direct_only | 0 | 0 | 0 / 0 |
| draft_component | 2 | 0 | 0 / 0 |
| economic_terms | 0 | 0 | 0 / 0 |
| extra_activity | 0 | 0 | 0 / 0 |
| fifo_mismatch | 0 | 0 | 0 / 0 |
| inclusive_fee | 2 | 1 | 0 / 0 |
| inexact_fee | 0 | 0 | 0 / 0 |
| intermediate_fee | 2 | 1 | 0 / 0 |
| liquid | 2 | 1 | 0 / 0 |
| outside_activity | 2 | 1 | 0 / 0 |
| partial_route | 0 | 0 | 0 / 0 |
| protected_middle | 0 | 0 | 0 / 0 |
| recovered | 2 | 1 | 0 / 0 |
| residual_fee | 2 | 1 | 0 / 0 |
| residual_suspense | 2 | 1 | 0 / 0 |
| reverse_txid | 2 | 1 | 0 / 0 |
| same_time | 2 | 1 | 0 / 0 |
| source_fee | 2 | 1 | 0 / 0 |
| two_sources | 3 | 1 | 0 / 0 |
| two_targets | 4 | 1 | 0 / 0 |
| untrusted_middle | 0 | 0 | 0 / 0 |
| untrusted_source | 0 | 0 | 0 / 0 |
| wallet_cycle | 2 | 1 | 0 / 0 |
| zero_activity | 2 | 1 | 0 / 0 |

Those books emit **2,640 claims**, **1,120 certificates**, and **80 blocked / 80
transient ids per engine**; native audits and non-events are zero for this suite.
Frozen tests retain **25 raw reconciliation cases**, **150 direct witnesses**,
**343 complete interpreter expectations**, and **106 wrapper/admission controls**.
The wrapper controls include list/dict/string iteration, duplicates, missing
fields, malformed JSON, nonfinite details and decoded surrogate ids.

### Verification and remaining stages

Verification uses the pinned Rust 1.99.1 toolchain, locked Python 3.11 environment,
`MATURIN_NO_INSTALL_RUST=1`, offline Cargo (including mutation children), and
disposable data/target roots. The environment is installed with
`uv sync --locked --python 3.11`. No push or PR is created.

- Formatting and workspace/all-target Clippy with warnings denied pass. Locked
  offline workspace tests excluding `kassiber-net` pass **740 tests**.
- All **1,178 mutation ids** have kills: 1,035 retained in four disjoint
  partitions (259/259/259/258), plus 143 new. Initial surviving witnesses were
  strengthened for inferred allocations, zero allocations, partial FIFO token
  reuse, merged origin credits, source deduplication, native non-event union and journal-anchor precedence.
  A nonbuilding string-iteration trial was corrected and rerun; neither it nor
  any surviving trial is counted as a kill. The final kill-id union equals the
  catalog, with no missing or extra ids. Catalog applicability also passes.
- All eleven complete-stage generated suites pass through the actual PyO3
  binding. Recorded replay passes **88 requests and 88 permutations**, with zero
  pre-request refusals and zero differences, across both retained recordings.
  The final fresh-process corpus separately passes **78 requests and 78
  permutations**, and all **78 snapshots** pass integrity checks.
- Focused census passes **52 tests and 21 subtests**, comparing interpretation
  for **78 requests**; fresh-process snapshot replay is enabled. Focused shadow
  passes **50 tests and 21 subtests**, comparing interpretation and the frozen
  Python composition for **76 requests**. The two structural read-budget cases
  stay in census because shadow deliberately computes the frozen flow again.
- Cutover/failure-injection tests pass **17 tests**, including evidence and
  interpretation discrepancies in both child modes and a caught-discrepancy
  session failure. Final wrapper regeneration checks pass separately.
- Broad custody/lifecycle/chronology/binding/inventory/contracts/mutation-runner
  tests report **592 passed, 2,685 subtests passed, one skipped, and four
  failures** (the eleven generated suites ran separately). The four are the
  unchanged daemon cases in `CustodyComponentDaemonSurfaceTests` and
  `CustodyGapDaemonProtocolAcceptanceTests`: their account-home owner-lock
  namespace is outside this sandbox's writable roots, as at the previous
  checkpoint. These failures are not reported as passing, and the stable owner
  lock boundary is not weakened. A diagnostic rerun confirms `internal_error` /
  `Read-only file system` for `~/.kassiber/run` in all four envelopes. Full census/custody-shadow shards and a
  nonsandboxed rerun of these four tests remain assigned to the orchestrator.
- Inventory classification, source contracts, mutation applicability and shard
  inventory validation pass. Manual second review covers the Rust/Python branch
  correspondence, full response inventory and absence of product Python writes.

The next stages are **gaps → quantity arbitration → finalized projection →
journal fusion with ledger**. In particular, the quantity stage still owns full
reviewed component/tax claim arbitration; this reconciliation compiler does not
replace it.


## Gap search checkpoint (R2d)

Branch `t3code/r2d-gaps` starts at `16234030`. The pure Rust `gaps` module
recomputes bounded missing-wallet discovery, scoring, conflicts, promotion
margins, matcher capacity and structured-source blockers. The PyO3 response lists
`gaps` as implemented; only `quantity` and `finalized_projection` remain skipped.
Census and custody-shadow compare the complete gap stage, including failures in
Python admission order. Python remains authoritative; Rust results never reach
stored state.

The snapshot's existing `gap_worklist.rows` and `large_book` flag are the exact
SQL-selected raw worklist. Rust consumes these rather than deriving a second
worklist from `transactions`. A SQL-backed test captures 67 selected rows from
304 raw observations, disables SQL, and proves exact stage parity. Snapshot v1
needs no new facts, version change or derived input. Loan/channel roles,
authored reservations and target-bearing interpreter claims are reconstructed
from the preceding Rust stages; accounting additionally excludes blocked ids.
Advisory candidates remain separate from authored components. Search can hold a
likely missing-wallet source in suspense, but only explicit review carries basis.

Pure helpers also port both fingerprint schemas, latest-review sibling conflict
and reopened-dismissal behavior, privacy-safe candidate display/downstream
impact, and seeded version-bound keyset pagination. Python retains SQL selection,
schema fallbacks, readiness, historical review lookups, review writes and bridge
activation. No dependencies, storage, product I/O or consent boundaries change.

### Bounds and branch dispositions

The production limits remain Python's: 50,000 normalized rows before bounded
worklist mode (threshold 1 for a SQL-selected large book), 256 source groups,
3 adjacent source legs, a 512-leg return pool, beam width 48, 16 beam return legs,
256 aggregate return legs, 24 ranked return groups per source, and 250 displayed
candidates. Indexed nearest-amount inspection stops at eight times the return
pool limit; indexed era inspection stops at 512 independently of that limit.
Candidate discovery/conflict stamping precedes display truncation. Capacity
remains ordinary result metadata with separate source blockers, never guessed
ownership or accounting evidence.

Every ported Python decision has a named mutation witness or this disposition:

- Production v1 supplies fixed typed matcher limits and a typed pagination
  adapter. Python kwargs type/negative-value validation and profile/argument
  type messages are unreachable at this raw snapshot seam. Reachable limit,
  cursor shape, version/ordinal/id expiry and paging decisions have witnesses.
- SQL querying, missing-table/schema fallbacks, readiness/status composition,
  historical review state and writes remain Python-owned adapters. They are not
  replaced by a Rust storage path. Raw latest review siblings, not a Python
  dismissal verdict, feed the pure dismissal helper.
- Nonempty sources and a positive source budget cannot produce an empty selected
  source worklist. The Python `_source_groups` helper's `max_groups` argument is
  unused; the caller enforces capacity before grouping. Defensive unsorted-pool
  handling is unreachable after explicit chronology sorting.
- Ordered-map intermediate traversal is equivalent only where subsequent
  explicit Python sort keys or minimum-id conflict roots settle output order.
  Chronology, same-time astral ids, amount-distance ties, score ties, group ids
  and source/return inspection ceilings retain independent witnesses.
- Trial GAP-012 (allowing zero at the initial positive-amount filter) is
  equivalent because the later nonpositive-principal guard rejects it. It is
  removed and never counted as a kill.
- Raw quantities use BigInt and Python's integer families; no raw fact passes
  through `codec::wide()`. CPython Unicode case/digit helpers and the corrected
  timestamp parser are reused. Century-scale elapsed times compare the exact
  rational half-ULP before integer truncation, avoiding an intermediate float's
  double rounding. Four independent Python counterexamples pin this behavior.
- Migration ids with lone surrogates remain in the complete interpreter output
  but cannot match UTF-8 raw worklist ids. The ignored-id projection omits only
  that impossible membership. Cursor JSON bytes separately retain Python's
  surrogatepass behavior for UTF-8/16/32, including raw pairs distinct from
  JSON-escaped pairs; invalid byte encodings still refuse.
- Display helpers consume generated normalized candidates. Invalid candidate
  end timestamps and non-string generated wallet labels are unreachable there;
  malformed raw rows are still admitted/rejected in Python order before ignore
  filters. Storage-generated review revisions are already selected raw facts.

### Mutation accounting

The catalog contains **1,315 entries: 1,178 retained and 137 new**. The table
names selectable Rust witnesses; each new entry builds successfully, passes its
unmutated baseline and fails its named test under mutation.

| Area | New entries | Named witnesses |
| --- | ---: | --- |
| Normalization, signals and calendar formatting | 28 | `gaps_search_vectors` |
| Bounded source/pool/era/beam search | 40 | `gaps_search_vectors` |
| Scoring, conflicts, fingerprints and safe display | 33 | `gaps_search_vectors`, `gaps_fingerprint_vectors`, `gaps_snapshot_vectors` |
| Stage composition, revisions and exact duration | 9 | `gaps_compute_vectors`, `gaps_review_revision_vectors`, `gaps_operation_vectors`, `gaps_search_vectors` |
| Ignored advisory/accounting populations | 7 | `gaps_ignored_ids_vectors` |
| Cursor codecs and seeded pagination | 20 | `gaps_cursor_vectors`, `gaps_seeded_pagination_vectors` |
| **New total** | **137** | **All killed** |

All 137 new ids and the 16 retained ids in edited boundary/helper files were
killed in this run. The other 1,162 retained kills are inherited from R2c and
were not rerun here. The checked final kill-id union covers every new id; no
survivor, nonbuilding trial or equivalent trial counts as a kill. Catalog
applicability checks all 1,315 entries.

### Generated, frozen and recorded evidence

All twelve suites compare exact gaps for **2,000 books per engine per suite**,
with independent Python and Rust scenario/signature counts, plus unordered-input
permutations. There are **515 scenarios, 24,000 books and 1,030 permutations**.
The checked-in [count matrix](../../tests/fixtures/custody_gap_counts.json)
contains every per-scenario output signature for both engines' assertions.
Signature fields are displayed candidates, accounting candidates, eligible count,
source blockers, completeness, limit kind and discovered candidates.

| Generator | Scenarios | Books per scenario per engine | Permutations |
| --- | ---: | --- | ---: |
| channels | 32 | 62–63 | 64 |
| components | 156 | 12–13 | 312 |
| evidence | 29 | 68–69 | 58 |
| fanout | 47 | 42–43 | 94 |
| gaps | 32 | 62–63 | 64 |
| graph | 8 | 250 | 16 |
| native | 55 | 36–37 | 110 |
| pipeline | 12 | 166–167 | 24 |
| reconciliation | 25 | 80 | 50 |
| recorded | 46 | 43–44 | 92 |
| safety | 39 | 51–52 | 78 |
| samourai | 34 | 58–59 | 68 |

The dedicated gap scenarios have these independent counts per engine. The
signature column uses the field order above; equality compares every field.

| Scenario | Books per engine | Gap signature |
| --- | ---: | --- |
| `blocked_competitor` | 62 | `2,1,1,0,1,None,2` |
| `clear_margin` | 62 | `2,1,1,0,1,None,2` |
| `cross_chain` | 62 | `0,0,0,0,1,None,0` |
| `cross_network` | 62 | `0,0,0,0,1,None,0` |
| `equal_competitors` | 63 | `2,0,0,0,1,None,2` |
| `era_capacity` | 62 | `0,0,0,1,0,boundary_worklist,0` |
| `excess` | 63 | `1,0,0,0,1,None,1` |
| `excluded` | 62 | `0,0,0,0,1,None,0` |
| `external_return` | 63 | `1,0,0,0,1,None,1` |
| `external_source` | 63 | `1,0,0,0,1,None,1` |
| `fee` | 63 | `1,0,0,0,1,None,1` |
| `inclusive_fee` | 63 | `1,0,0,0,1,None,1` |
| `large_empty` | 62 | `0,0,0,0,0,boundary_worklist,0` |
| `large_no_returns` | 62 | `0,0,0,0,0,boundary_worklist,0` |
| `large_structured` | 62 | `1,0,0,1,0,boundary_worklist,1` |
| `large_untyped` | 62 | `1,0,0,0,0,boundary_worklist,1` |
| `loan_ignored` | 62 | `0,0,0,0,1,None,0` |
| `long_horizon` | 63 | `1,0,0,0,1,None,1` |
| `missing_wallet` | 63 | `1,0,0,0,1,None,1` |
| `missing_wallet_and_disposal` | 62 | `1,0,0,0,1,None,1` |
| `multiple_wallets` | 63 | `1,0,0,0,1,None,1` |
| `postmix_fan_in` | 63 | `1,0,0,0,1,None,1` |
| `return_capacity` | 62 | `0,0,0,1,0,boundary_worklist,0` |
| `same_time` | 62 | `0,0,0,0,1,None,0` |
| `separate_eras` | 63 | `1,0,0,0,1,None,1` |
| `source_capacity` | 62 | `1,0,0,1,0,boundary_worklist,1` |
| `split_returns` | 63 | `1,0,0,0,1,None,1` |
| `split_sources` | 63 | `1,0,0,0,1,None,1` |
| `unicode_labels` | 62 | `1,0,0,0,1,None,1` |
| `untyped` | 63 | `1,0,0,0,1,None,1` |
| `whirlpool_policy` | 63 | `1,1,1,0,1,None,1` |
| `whirlpool_tx0` | 63 | `1,1,1,0,1,None,1` |

Whirlpool policy/tx0 cases each produce one accounting candidate per book;
blocked competitors and clear score margins preserve the separate accounting
search. Coinjoin-only quarantines remain advisory. Source, return and era
capacity cases independently assert source-only holds. Realistic books cover
split sources/returns, postmix fan-in, missing wallets followed by disposals,
external-origin disqualification and long horizons.

Frozen Python fixtures pin **173 searches, 30 compute controls, 396 fingerprint
pairs, 7 review revisions, 31 display rows, 98 cursors and 13 seeded pages**.
Another **96 operation snapshots** pin ignored populations, full responses and
admission error families. The adversarial search has **60,301 rows and 300
structured sources**; both engines finish under the 20-second smoke ceiling.
Rust additionally pins 300 blockers, 251 discovered and 250 displayed candidates.

### Verification and remaining work

Checks use Rust 1.99.1, locked Python 3.11 (`uv sync --locked --python 3.11`),
`MATURIN_NO_INSTALL_RUST=1`, offline Cargo including mutation children, and
disposable books/target roots. No push or PR is created.

- Workspace formatting, workspace/all-target Clippy with warnings denied and
  locked offline workspace tests excluding `kassiber-net` pass **750 tests**.
- The complete generated/permutation gap suite passes **17 tests**. The final
  rebuilt-binding run passes **36 tests**, including all twelve generators,
  frozen regeneration, interpreter wrapper/refusal controls and child/session
  discrepancy injection. Inventory/contracts and mutation-runner contracts pass
  **13 tests and 14 subtests**. CPython text regeneration, source contracts,
  shard inventory and mutation applicability checks pass.
- Recorded integrity and Rust replay pass **73 census snapshots and 6 shadow
  snapshots: 79 requests and 79 permutations**, with zero differences and zero
  pre-request refusals.
- Focused census and shadow each pass **54 tests and 2 subtests** (two structural
  read-budget cases deselected), comparing gaps for **6 requests each**. The
  broader census passes **180 tests and 52 subtests**, comparing **67 requests**.
- Focused Python custody modules pass **160 tests and 4 subtests**. Two daemon
  tests initially fail because their stable account-home owner-lock namespace
  lies outside the sandbox's writable roots; both pass on the authorized rerun
  with that access. The product owner-lock boundary is unchanged.
- Failure injection proves gaps discrepancies are recorded in both child modes
  and remain session failures when caught. Manual second review covers Python
  branch correspondence, bounded scans, exact integers/timestamps, raw snapshot
  worklists, response inventory and the absence of production Rust writes.

Full census/custody-shadow shards remain assigned to the orchestrator; they were
not run here. The next stage is **quantity arbitration**, followed by finalized
projection and journal fusion with `ledger`. Quantity still owns the complete
reviewed-component/tax-claim arbitration and downstream report blockers.

## Quantity primitive checkpoint (R2e, partial)

Branch `t3code/r2e-quantity` starts at `23c059dc`. This checkpoint ports
`custody_quantity.py`'s arbitration kernel, `allocate_msat_fifo`,
`compare_wallet_balances`, and the quantity issue, custody-decision and economic
relation identity helpers. It does **not** implement the complete `quantity`
stage. PyO3, census and custody-shadow continue to list `quantity` and
`finalized_projection` as skipped. A regression pins that inventory.

The new pure `crates/custody/src/quantity` module validates observations and
claim bundles, isolates malformed bundles as source suspense, splits source
slices, rejects incompatible authoritative interpretations and equal-priority
overlap, rejects transitive destination collisions, enforces atomic completeness,
and emits balanced wallet/fee/external/suspense/conflict postings. Integer msat
arithmetic uses `BigInt` throughout, including capacities, coordinates and
balances above `i128`. No new code calls `codec::wide()`. CPython case,
whitespace, Unicode decimal and compact JSON helpers preserve identities and
error families. Arbitration retains Python's lexical ordering of recorded
timestamps; it introduces no timestamp parser. The inherited corrected parser
continues to own chronological admission in earlier stages.

FIFO returns source and sink offsets, allocated quantity and both ordered
residual lists. It admits source buckets before sink buckets, requested quantity
and capacity checks, retaining Python's allocation codes, messages and details.
The JSON test seam accepts the finite iterable shapes represented in fixtures;
arbitrary Python objects and custom iterators cannot reach a Rust snapshot.
Wallet differences compare exact derived postings with current engine balances
without granting the engine authority. Only one distinct known non-event reason
may explain a difference. The hash helpers match **44 issue ids and 28 stored
decision/relation ids**, including every preimage byte; the 12 component
commitment ids remain covered by the earlier component port. Hash fixtures are
embedded so cached test binaries do not depend on deleted mutation scratch paths.

### Evidence and branch dispositions

The frozen CPython primitive corpus contains **374 arbitration cases** (320
successful projections and 54 refusals), **519 distinct claim-constructor
controls**, **80 FIFO cases** and **36 wallet comparisons**. It captures the
existing Python quantity and runtime behavior pins independently of Rust, then
adds isolated constructor guards, semantic metadata disagreements, malformed
bundles, exact boundary capacities and integer quantities beyond machine size.
The fixture regeneration check executes the original Python assertions too.

The separate generator compares **2,000 projections and 2,000 permutations**.
Both engines independently assert this per-scenario count matrix; signatures
and exact counts are checked in
[`custody_quantity_primitive_counts.json`](../../tests/fixtures/custody_quantity_primitive_counts.json).
These are primitive inputs, separate from raw v1 snapshots and stage outputs.

| Scenario | Books per engine | Decision states |
| --- | ---: | --- |
| `unmatched` | 118 | 1 external presumed |
| `fee_only` | 118 | No principal decision |
| `fee_inclusive` | 118 | 1 external presumed |
| `native` | 118 | 1 internal verified |
| `partial_native` | 118 | 1 internal verified, 1 external presumed |
| `source_conflict` | 118 | 1 conflicting |
| `equal_priority` | 118 | 1 conflicting |
| `destination_conflict` | 118 | 2 conflicting |
| `atomic` | 118 | 1 internal reviewed, 1 custody suspense |
| `atomic_partial` | 118 | 2 conflicting |
| `retained` | 118 | 1 internal reviewed |
| `reviewed_external` | 117 | 1 external confirmed, gift subtype retained |
| `external_fee` | 117 | 1 external confirmed fee |
| `negative_balance_hold` | 117 | 1 custody suspense |
| `domain_reject` | 117 | 1 custody suspense |
| `cross_rail` | 117 | 1 internal verified |
| `one_msat` | 117 | 1 internal verified |

The negative-balance case supplies an explicit interpreter hold against an
observed debit without prior funding. It proves arbitration preserves that hold;
it does not claim a new automatic negative-balance detector. Frozen replay also
compares all 374 cases and permutes all 320 successful projections. Refusal
inputs retain admission order.

Every ported Python decision has a mutation witness or this disposition:

- The Python dataclasses are constructed before arbitration. Their JSON forms
  are private intermediate inputs, not new raw facts or snapshot fields.
  Constructor tests isolate guards otherwise masked by later conservation or
  posting checks, preserving `ValueError` families without counting a later
  failure as a constructor witness.
- Enum membership admits only priorities 10, 20 and 40. Every nonempty bundle
  therefore has a suspense-compatible minimum. The Python empty-priority
  fallback and Rust `unwrap_or(40)` cannot run for admitted claims. Dataclass
  state/priority, target, subtype, destination and bundle guards are independently
  exercised.
- Source boundaries are a sorted set, so Python's adjacent `start == end`
  branch cannot run. Every target-bearing decision has a nonempty selected id;
  overlap-cluster missing/empty-id filters are equivalent after admission.
  Likewise, targetless internal posting failures are unreachable after claim
  construction except the explicitly permitted reviewed retained location.
- Exact integer parsing independently rejects nonintegers through `BigInt`'s
  grammar. Removing only its preliminary JSON-type guard is equivalent, so the
  surviving trial was excluded, not counted as a kill. Observation and FIFO
  amount/type guards have independent witnesses.
- The final conservation assertion cannot fail for valid inputs with the
  preceding arithmetic intact. Posting, fee, origin, domain and target mutations
  exercise the invariant through actual regressions; no fabricated unbalanced
  successful input is used to claim a separate assertion mutation.
- Complete projections bind every wallet posting to its observation; the
  balance helper's missing-observation lookup is defensive and cannot occur
  in the stage. Python custom object/iterator behavior and non-string reason
  sets are outside the typed, JSON-representable caller contract.
- The remaining quantity runtime branches are **unported**, not unreachable:
  full reviewed conversion/economic terms and diagnostics, native audit virtual
  sources/targets, gap-hold validation and dismissal/capacity composition,
  rejected-event/interpreter issues, scoped eligibility barriers and canonical
  internal-transfer rows still belong to the next quantity checkpoint.

### Mutation catalog and verification

The catalog has **1,431 entries: 1,315 retained and 116 new**. Named witnesses
are selected independently; every counted kill requires a successful baseline,
a building mutant and a failing named test. Equivalent trials and failed
baselines are excluded.

| Area | New entries | Named witnesses |
| --- | ---: | --- |
| Exact observation admission, domain and boundary arithmetic | 23 | `quantity_arbitration_vectors` |
| Claim construction, bundle validation, source/target arbitration and postings | 65 | `quantity_claim_constructor_vectors`, `quantity_arbitration_vectors` |
| FIFO admission, capacity, offsets and residuals | 17 | `quantity_fifo_vectors` |
| Wallet differences and explanations | 5 | `quantity_balance_vectors` |
| Quantity identity preimages | 6 | `quantity_issue_ids_match_all_r0_vectors`, `quantity_store_ids_match_all_r0_vectors` |
| **New total** | **116** | |

All **1,431 ids have named-test kills in this run**, including all **1,315
inherited entries** in four disjoint runs of 329, 329, 329 and 328. New-rule
rechecks use the final isolated constructor/semantic/FIFO witnesses; no survivor,
nonbuilding mutant, failed baseline or equivalent trial counts as a kill.

Verification uses Rust 1.99.1, locked Python 3.11, `MATURIN_NO_INSTALL_RUST=1`,
offline Cargo including mutation children, and disposable data/target roots.
The Python implementation, snapshot schema, production authority and persistence
remain unchanged. Full census/custody-shadow shards remain assigned to the
orchestrator; the focused lanes deliberately report zero quantity comparisons
until complete runtime composition is implemented.

- Workspace formatting, all-target workspace Clippy with warnings denied and
  locked workspace tests excluding `kassiber-net` pass **756 tests**.
- The primitive Python suite passes its frozen regeneration, generated counts,
  2,000 permutations, 374 recorded primitive replays, 320 successful replay
  permutations and skipped-stage inventory guard. The inherited full generator
  suite passes **17 tests**, retaining **24,000 books and 1,030 permutations**
  across its twelve raw snapshot suites and 515 scenarios.
- Focused quantity/runtime/view/handler/hash Python modules pass **101 tests
  and 2,605 subtests**. Focused census and custody-shadow each pass **84 tests
  and 22 subtests**; each compares the four implemented stages on **40 builds**,
  and shadow also completes **40 frozen Python comparisons**.
- Raw recorded replay passes **159 requests and 159 permutations**: the
  retained 73 census and 6 shadow recordings plus 40 fresh recordings from each
  focused lane. There are zero differences and zero pre-request refusals.
- Inventory/contracts and mutation-runner contracts pass **13 tests and 14
  subtests**; shard inventory and all 1,431 mutation applicability checks pass.
  The shared JSON-feature check retains **145 byte-identical ledger outputs**.
- Manual second review checks branch correspondence, exact msat, CPython
  identity preimages, admission order, overlap/atomic fail-closed behavior,
  unchanged raw snapshot admission and the absence of production Rust writes.

Remaining work is the complete quantity runtime composition listed above,
followed by **finalized projection** and journal fusion with `ledger`.

## Canonical transfer row checkpoint (R2e, partial)

Branch `t3code/r2e-quantity-2`, based on `7969fa8e`, additionally ports
`canonical_internal_transfer_rows` to the private Rust quantity primitive seam.
It emits only targeted internal verified/reviewed decisions, uses anchor
transaction ids, preserves explicit null/empty wallet labels, and retains
component/bundle provenance only when truthy. Basis readiness compares the
whole finalized decision, rather than its claim id. An exact custody edge
therefore stays visible behind a prior basis barrier. Amount subtraction and
numeric sort ties use `BigInt`; timestamps retain Python lexical ordering.
Missing observation lookup returns Python's `KeyError` family, though complete
arbitration projections always bind both sides. Non-JSON mapping implementations
remain outside this private fixture seam.

The frozen CPython corpus contains 12 scenarios. Each generated case includes
a separate exact companion edge so exclusion scenarios cannot pass vacuously.
Both engines assert these per-scenario active row totals across 1,200 inputs:

| Scenarios | Inputs each | Active rows each |
| --- | ---: | ---: |
| verified, reviewed, blocked, labels, empty_ids, anchor_ids, liquid, wide | 100 | 200 |
| retained, external | 100 | 100 |
| slice, sort | 100 | 400 |
| **Total** | **1,200** | **2,600** |

The generator also reverses observations, decisions and eligible decisions for
all 1,200 inputs. Frozen vectors cover nonzero source offsets, exact quantities
above machine width, numeric sort ties and slices sharing a selected claim id
with different eligibility. The private JSONL driver gains `transfers`; raw
snapshot v1 and Python production behavior are unchanged.

The mutation catalog adds 31 `QT-*` entries, each naming
`quantity_transfer_vectors`, for a total of 1,462 entries. They pin state and
target filtering, anchors, labels, offsets, full-decision eligibility, optional
provenance, sorting and every emitted field.

Quantity remains skipped in PyO3, census and custody-shadow. Reviewed component
and conversion compilation, native augmentation, gap-hold composition, issues
and scoped tax barriers still require porting before quantity can be enabled.
Rust results never reach stored state. This checkpoint does not claim complete
runtime quantity parity.

Verification at this boundary:

- Rust 1.99.1 formatting and workspace/all-target Clippy with warnings denied
  pass. Offline locked workspace tests pass **775 tests**, including the
  network crate's isolated local test listeners.
- The quantity primitive suite passes **5 tests**, retaining the earlier
  2,000 projections/permutations and frozen replays while adding the 1,200
  transfer inputs/permutations above. The broader retained generator run
  passes **37 tests**, including the twelve-suite gap comparison's **24,000
  books and 1,030 permutations** with zero differences.
- The quantity/runtime/view/handler modules plus primitive suite pass **96
  tests and 36 subtests**. Focused census and shadow each pass **91 tests and
  36 subtests**, comparing all four implemented stages on **40 builds** with
  zero mismatches. Shadow also completes **40 frozen Python comparisons**.
  Quantity correctly remains at **0 compared, 40 skipped** in each lane.
- Read-only replay of the retained four recording directories compares
  **159 requests and 159 permutations**, with zero differences or pre-request
  refusals. New recordings are separate: **40 census and 40 shadow snapshots**
  under `target/r2e2-verify/{census,shadow}-snapshots` in this worktree.
- Inventory/contracts and mutation-runner contracts pass **13 tests and 14
  subtests**. Manual second review checks exact arithmetic, label presence,
  source rather than target attributes, full-decision membership, stable
  lexical/numeric sorting and the unchanged raw-only/persistence boundary.

No Python reference bug was identified in this checkpoint.

All **1,462 catalog ids have named-test kills in this run**: the unchanged
**1,431 inherited entries** and all **31 new entries**. Every counted result
has a passing unmutated named test, a building mutant and a failing named test.
The coverage audit retains 120 completed inherited kills from the initial runs,
checks the other 1,311 in twelve disjoint runs, and independently rechecks all
31 transfer mutations against the final arbitration-produced vectors. Interrupted
trials contribute no result. There are no survivors, nonbuilding mutants or
failed baselines. The local audit and witness map are
`target/r2e2-verify/mutation-summary.json`; verification logs stay under that
ignored artifact root. New snapshots also replay as **80 requests and 80
permutations**, with zero differences or pre-request refusals.

## Reviewed compilation and native augmentation checkpoint (R2e, partial)

Branch `t3code/r2e-quantity-3`, based on `180edd62`, ports
`compile_component_quantity_claims` and `compile_verified_native_claims` as
private, comparison-only quantity primitives. Reviewed compilation includes
inferred and explicit allocations, transactionless FIFO routes, source/target
cursors, observation fees, reviewed residuals, high-end conversion slices,
economic conversion records and direct payouts. Native augmentation includes
virtual channel-close sources, exact audit admission and source conservation,
aggregate observed/synthetic targets, alias preflight, connected invalidity
closure, reservations and implicit-wallet-delta suspense.

Every msat operation uses `BigInt`. Synthetic observations share the evidence
field/commitment constructor, then use exact quantity admission; they do not
pass through the earlier evidence stage's machine-width validator. Earlier
stage admission is unchanged. CPython uppercase and integer conversions,
lexical timestamps, error families and component compiler diagnostics are
compared independently. The private reviewed seam accepts ordered observation
pairs so dictionary admission order survives JSON decoding. No intermediate
compiler operand is added to snapshot v1.

| Frozen Python operands | Successful outcomes | Refusals | Claims |
| --- | ---: | ---: | ---: |
| Reviewed components/conversions (83) | 39 | 44 | 51 |
| Native augmentation (100) | 96 | 4 | 13 |
| **Total (183)** | **135** | **48** | **64** |

Successful native compilation may deliberately emit diagnostics and no claims.
The generator therefore gives each scenario a separately checked positive
companion. Every primary result is compared too; companions cannot mask a
changed refusal, missing issue, orphan synthetic observation or missing claim.
Both engines assert the checked-in active-count manifest for every scenario.

| Compiler operands | Frozen scenarios | Distinct generated inputs per engine |
| --- | ---: | ---: |
| Reviewed | 83 | 324 |
| Native | 100 | 336 |
| **Total** | **183** | **660** |

The original twenty-variant loop repeated inputs: its 3,660 primaries contained
only **660 distinct operands**, and its positive companions added no distinct
inputs. The corrected generator discards repeated variants, and the comparison
deduplicates operands across scenario labels before invoking either engine.
It compares **660 distinct compiler inputs per engine**, plus the independently
re-evaluated reversed operands. The previous “7,320 compiler calls” claim counted
repetitions and is withdrawn. Variants use four exact integer widths, including
quantities above `i128`, and lexical offset timestamps. Frozen
controls include duplicate pending allocation ids, partial and native
conversion residuals, negative slice admission, explicit/leg-derived external
subtypes, Unicode proof identities, virtual-source contradictions and a native
source/target graph that requires more than one invalidity-closure pass.

The catalog adds **335 entries**. Each counted entry has a passing unmutated
named test, a building mutant and a failing named test. Twelve equivalent or
defensive trials are excluded; interrupted trials and a failed
baseline contribute no result. The affected witnesses were rechecked with a
fresh target root. The audit and witness map are
`target/r2e3-verify/mutation-summary.json`.

| Mutation area | New entries | Named witnesses |
| --- | ---: | --- |
| Reviewed admission, routes, conversion and output fields | 157 | `quantity_reviewed_compilation_vectors` |
| Native admission, augmentation, contradictions and output fields | 149 | `quantity_native_augmentation_vectors` |
| Arithmetic, ordering, defaults, identities and constructors | 29 | Both compiler witnesses above |
| **New total** | **335** | **All killed** |
| Inherited catalog rerun | 1,462 | **All killed** |
| **Catalog total** | **1,797** | **All killed** |

Verification uses Rust 1.99.1, locked Python 3.11,
`MATURIN_NO_INSTALL_RUST=1`, offline Cargo and disposable data roots:

- Workspace formatting and all-target Clippy with warnings denied pass.
  Workspace tests pass **777 tests**: 759 with `kassiber-net` excluded and 18
  in that crate's separately run isolated local-listener tests.
- The inherited generator suite passes **31 tests**, including the twelve
  raw-snapshot suites' **24,000 books and 1,030 permutations**, with zero
  differences. The final quantity primitive/compiler suite passes **7 tests**,
  including frozen regeneration, activity counts and reversed operands.
- Inventory/contracts, mutation-runner contracts and snapshot checks together
  with the quantity suite pass **26 tests and 14 subtests**. All **1,797**
  catalog applicability checks pass.
- Focused census and shadow each pass **91 tests and 36 subtests**, comparing
  four implemented stages on **40 builds** with zero mismatches. Shadow also
  completes **40 frozen Python comparisons**. Each lane retains **0 quantity
  comparisons and 40 quantity skips**.
- Read-only replay compares the retained **159 requests and 159 permutations**
  with zero differences or pre-request refusals. Fresh recordings remain under
  this worktree's `target/r2e3-verify/{census,shadow}-snapshots`.
- Manual second review checks Python branch correspondence, admission order,
  exact source/target coordinates, conversion terms and defaults, native proof
  preimages, multi-pass contradiction closure and the raw-only/persistence
  boundary.

Fresh recordings additionally replay as **80 requests and 80 permutations**
with zero differences or pre-request refusals.

Branch dispositions:

- The flattening pass has already checked allocation endpoints and quantity
  conservation and discarded nonpositive allocations before the claim loop.
  Repeating those guards there is defensive. Its returned sources are observed
  by construction; a transactionless source cannot escape its token pools.
- Source/target slice constructors retain their own refusal witnesses,
  including malformed conversion coordinates. The complete claim constructor
  also rejects a conversion whose unobserved retained sink would otherwise
  combine an internal state with external destination classification. The
  preceding target-equality/retained-destination assertion is defensive rather
  than a separate reachable constructor failure.
- Inferred allocation ordinals are not consumed after inference; changing
  that intermediate field alone is equivalent. Sorting explicit allocations
  and FIFO/source/target cursor behavior have separate witnesses.
- Alias ownership is installed after preflighting a whole slot. A later slot
  cannot have the same slot identity, so the same-slot owner guard is
  defensive. Default-false fee-inclusion fields and a default-zero synthetic
  inbound fee cannot be tested by merely omitting those fields; counted
  mutations change their actual values instead.
- Non-JSON custom mappings/iterators and lone-surrogate strings are outside
  this typed private fixture seam. No shell, filesystem, snapshot or product
  interface is added.

Quantity stays skipped in PyO3, census and custody-shadow. Gap-hold composition,
whole-state issues (including authored-component evidence drift/fail-closed
composition), scoped tax barriers, finalized projection and journal fusion
remain unported. Python stays authoritative and Rust results never reach
stored state. No Python reference bug was identified in this checkpoint.

## Complete quantity runtime checkpoint (R2e)

Branch `t3code/r2e-quantity-4`, based on `a4151d44`, ports the remaining
quantity runtime composition: advisory gap holds, whole-state issues and scoped
tax eligibility. `quantity` now participates in the PyO3, census and custody-shadow
stage inventories. Only `finalized_projection` remains skipped. Python remains
authoritative, Rust results never reach stored state, and snapshot v1 remains
raw-only. Finalized tax projection and journal fusion are outside this checkpoint.

Gap candidates validate their ordered boundary ids, observations, profile,
identity, asset/chain/network, wallets and exact quantities before producing
independent holds. Current dismissal fingerprints suppress only that candidate;
capacity holds select only an affected outbound principal. Compiler disagreements
keep both boundaries in review and emit source suspense, never a destination edge.
Overlapping suggestions retain independent diagnostic/return identities while
source claims are deduplicated by their exact slices.

Runtime composition includes authored-active invalidity, author-local evidence
drift and compiler failures; canonical rejections; malformed bundles; native
audit and interpreter blockers; and unresolved decisions. Compiler `TypeError`
and `ValueError` become `custody_component_claim_compile` issues with their
original messages, rather than leaking the exception family as the issue reason.
Uncaught families preserve Python admission order. Diagnostic issues remain
unquantified where source decisions already quantify the same suspense.

Tax barriers isolate profile/network/exposure/unit while sharing Bitcoin basis
across rails. Each pool keeps its earliest canonical event and all roots at that
event. Finalized sibling slices remain eligible; a different event at the same
lexical timestamp fails closed. Native/gap issues directly block their named
observations. Rejected rows resolve only pool scope, without granting event
identity; a truly unscoped contradiction affects every observed pool.

All msat arithmetic uses `BigInt`. CPython case and lexical timestamp semantics
are retained. Migration quarantines keep nonfinite diagnostic values and
surrogate identifiers in the CPython value representation through serialization.
A surrogate identifier cannot match a UTF-8 raw row; its temporary absent-row
lookup supplies neither quantity nor a temporal barrier. Exact ids, reasons and
details are restored before output. The private primitive fixture serializer
uses the existing `$float` tags for nonfinite details; this is not a snapshot
field or a product interface.

### Generated and frozen evidence

The frozen runtime corpus captures the existing Python behavior assertions and
adds malformed-boundary, capacity/dismissal, blocker JSON and compiler-diagnostic
controls. Every generated scenario has a separately compared positive companion;
its primary result is also compared, including refusals. Both engines assert the
checked-in per-scenario activity manifest, and reversed operands are independently
re-evaluated by Python.

| Runtime operand family | Frozen scenarios | Distinct generated inputs per engine |
| --- | ---: | ---: |
| Gap-hold composition | 97 | 384 |
| Scoped tax barriers | 68 | 256 |
| Whole-state composition | 63 | 252 |
| Component runtime diagnostics | 83 | 324 |
| **Total** | **311** | **1,216** |

The original 6,220 primaries contained **1,216 distinct operands**. Its four
companions were already in that set. The corrected comparison invokes each
distinct operand once, retaining scenario labels for the activity manifest;
reversed operands are independently evaluated and deduplicated separately.
The previous “12,440 calls per engine” claim counted repetitions and is withdrawn.
Companion activity is diagnostic evidence for its own operand, never activity
attributed to a raw snapshot seed.

The 106 inherited admission/migration wrapper cases also compare complete
quantity, including CPython diagnostic scalars, surrogate identities and refusal
families. The inherited twelve raw snapshot suites compare quantity on
**24,000 books and 1,030 permutations**. Fourteen deliberately empty controls
remain parity checks but are excluded from activity claims; the checked-in
`custody_quantity_inactive_controls.json` names them. Every other scenario must
produce its own decisions, issues or refusal, independently in both engines.
A named regression proves that a vacuous scenario fails this assertion. One
additional malformed migration request runs through the same raw comparison
loop, so its refusal branch is exercised and counted without changing the
inherited 24,000 books or 1,030 permutations.

### Mutation catalog and verification

At the original R2e checkpoint the catalog contained **1,918 entries: 1,797 inherited and 121 new**. The independent-review corrections below supersede that inventory. New entries
name `quantity_gap_hold_composition_vectors`,
`quantity_scoped_tax_barrier_vectors`, `quantity_whole_state_vectors`,
`quantity_component_runtime_vectors` and `quantity_runtime_wrapper_vectors`.
Six inherited recipes are recontextualized for the richer compiler diagnostics.
Two of those now change the reachable zero-offset boundary: omitting the local
range check alone is equivalent to the later claim constructor's range check,
and is not counted as a mutation kill.

All **1,918 mutations are killed**, including the complete **1,797-entry inherited
catalog**. Every counted result retains a passing unmutated named test, a building
mutant and a failing named witness.
Interrupted trials, equivalent omissions, nonbuilding mutants and failed
baselines contribute no result. Each concurrent batch owns its target directory;
targeted reruns build the test binary that contains the catalog's named witness.
The per-entry audit is `target/r2e4-verify/mutation-summary.json`.

Verified integration evidence at this boundary:

- Focused census and custody-shadow each pass **91 tests and 36 subtests**.
  Each compares quantity on **40 builds**, with **zero mismatches**; shadow
  also completes **40 frozen Python comparisons**. Finalized projection stays
  at **0 compared, 40 skipped** in each lane.
- Read-only retained replay compares **159 requests and 159 permutations**,
  requires quantity and reports **zero differences and pre-request refusals**.
  Fresh worktree recordings replay as **80 requests and 80 permutations**, also
  with zero differences or pre-request refusals.
- The enabled-stage generator run passes **41 tests**, including the inherited
  raw generators, complete quantity comparisons, primitive/compiler fixtures,
  scenario activity and permutations. Harness/snapshot/mutation contracts pass
  **44 tests and 14 subtests**, including quantity discrepancy injection.
- Rust formatting and workspace all-target Clippy with `-D warnings` pass.
  Workspace tests pass **782 tests** (764 ordinary and 18 isolated socket tests).
  Full census/shadow shards remain assigned to the orchestrator.

No Python reference bug has been identified. Remaining work is finalized
projection, followed by journal fusion with `ledger` after its own parity gate.

## R2e independent-review corrections

The live census/shadow harness now waits for Python's stage outcome before
querying the atomic Rust result. An implemented-stage Python refusal is compared
by code even after gaps has emitted. Matching refusals preserve Python's original
exception; earlier outputs absent from an atomic refusal are explicitly skipped.
Unported finalized projection remains skipped. Successful and refused builds
must account for every stage, and session reporting rejects incomplete inventories.
The late-refusal regression covers Rust success, matching/mismatching refusal
codes and both live modes.

A deliberately patched Python stage must explicitly declare
`injected_stage_failure(stage, exception)` in its test. The annotation is scoped
to the exact exception instance and failed stage; equal messages, other stages
and later builds retain strict comparison. Such a build records `outcome` and
`reason` as `injected`, with every stage skipped. Session reporting validates
that inventory and counts injected builds separately. The snapshot recorder's
intentional `RuntimeError("boom")` uses this annotation; it is not a Rust refusal.

Tax eligibility admits only Python's four finalized states. Gap identity rejects
an empty asset at Python's identity-validation position, with empty compiler
details. Reviewed source slices reject empty observation hashes before target
validation. Component conservation, allocations, chronology, anchor coverage and
native support retain exact integers instead of narrowing component amounts to
`i128`. No projection code or production Python accounting rules change.

The local correction report is [custody-r2e-quantity-review.md](custody-r2e-quantity-review.md).


## Finalized row primitive checkpoint (R2f, partial)

The local `t3code/r2f-projection` checkpoint starts at `9d2ab49c` and ports the
first finalized-projection primitives in `crates/custody/src/finalized_projection.rs`:

- `FinalizedTaxProjection` row marker, unique id and intra-pair membership
  validation, retaining Python's row-before-pair admission order and errors;
- slice, fee and direct-payout row identities with unbounded integer coordinates;
- original transaction/anchor fallback and base-row construction, including
  unchanged raw fields and timestamps, canonical observation overrides and
  falsey external-id/raw-JSON defaults;
- chain-order evidence captured against the original graph and quantity
  commitments before changing the id or principal; nested graph parents retain
  strict Python `int` outpoint semantics, exclude booleans/floats/strings and
  preserve unbounded nonnegative indices;
- fee-only rows, with zero principal, exact fee, anchor linkage, cleared absolute
  fiat values and the exact-authority privacy-marker exception.

The current-observation predicate is deliberately distinct from the full
evidence authority predicate: the former checks the stored version and current
hashes, then physical transfer scope. Additional evidence-stage admission gates
must not change the Python row builder. Graph commitment failure short-circuits
raw amount parsing; committed raw amount and fee failures retain Python's
`TypeError`/`ValueError` families. No raw amount uses bounded or float conversion.

This is a **partial comparison checkpoint**. `finalized_projection` remains
explicitly skipped by PyO3, census and custody-shadow. Its production comparison
count is zero. `projection-primitives` is an unpackaged JSONL example using
separate typed primitive operands; no intermediate output is added to snapshot
v1. No Rust result reaches persistence or the tax engine. Python remains the
production reference.

### Frozen and generated evidence

`tests/custody_projection_primitives.py` independently computes the Python
outputs. It covers malformed row/pair admission, coercion and duplicate ids,
source/anchor precedence, rowless observations, falsey defaults, exact raw
quantities, lexical timestamps, fee privacy, stale provenance and nested parents.
Every scenario has an independently compared active companion, including refusal
controls. Primary outputs and refusal families are compared separately; the
companion cannot mask a difference. Both engines assert the checked-in per-scenario
activity manifest in `tests/fixtures/custody_projection_counts.json`.

| Primitive family | Frozen scenarios | Generated batches per engine | Active results per engine |
| --- | ---: | ---: | ---: |
| Projection row contract | 15 | 300 | 1,000 |
| Exact row identities | 2 | 40 | 240 |
| Base rows | 10 | 200 | 800 |
| Fee-only rows and original chain proof | 15 | 300 | 1,140 |
| **Total** | **42** | **840** | **3,180** |

The generated batches make **1,680 calls per engine**, plus **1,680 dictionary
order permutations**. Fee amounts and row-id coordinates cross machine ranges;
canonical wallet/time overrides and untouched raw row fields vary independently.
Dictionary permutations change lookup insertion order, preserving transaction
versus anchor precedence.

### Remaining finalization work

The next slice is principal slice construction, including derived exact pricing
from absolute fiat values. Reviewed payout admission, timestamp handling and
settlement rows follow. Full composition still needs connected-path blocking,
fee allocation and deduplication, selected MOVE/external/inbound rows, scoped
basis barriers, issue/quarantine composition and reviewed conversion metadata.
Only after those agree may finalized projection enter the stage inventories.
Journal fusion with `ledger` remains outside R2f. No Python reference bug has
been identified in this checkpoint.

### Mutation catalog and verification

The catalog now has **1,955 entries: 1,918 inherited and 37 new**. The complete
inherited catalog was rerun in independent disposable source/target batches.
The final new catalog was checked with the shared scratch-tree runner. Each
counted entry has a passing unmutated baseline, a building mutant and a failing
named witness. One initial uniqueness mutation did not build because removing
insertion left a set's type ambiguous; an explicit `BTreeSet<String>` annotation
makes the omission independently testable. Its successful rerun replaces that
trial; the nonbuilding attempt contributes no kill.

| New named witness | Killed mutations |
| --- | ---: |
| `projection_contract_vectors` | 5 |
| `projection_identity_vectors` | 3 |
| `projection_base_row_vectors` | 6 |
| `projection_fee_row_vectors` | 23 |
| **Total** | **37** |

All **1,955 entries are killed**. The local per-entry audit is
`target/r2f-verify/mutation-summary.json`; initial batch records and the shared
runner's final projection logs are retained beside it.

| Verification | Result |
| --- | --- |
| Inherited raw generators | 24,000 books and 1,030 permutations; zero differences in implemented stages |
| New primitive generators | 42 scenarios, 840 batches, 1,680 calls and 1,680 dictionary permutations per engine; every scenario active |
| Retained read-only replay | 159 requests and 159 permutations; zero differences or pre-request refusals |
| Fresh worktree replay | 100 requests and 100 permutations; zero differences or pre-request refusals |
| Focused census | 98 tests; 50 comparisons per implemented stage; zero mismatches |
| Focused custody-shadow | 98 tests; 50 comparisons per implemented stage and 50 frozen Python comparisons; zero mismatches |
| Finalized projection inventory | 0 compared, 50 skipped in each focused lane |
| Generator/primitive pytest lane | 25 tests passed |
| Snapshot, shadow, mutation and Rust-port contracts | 41 tests and 14 subtests passed |
| Rust workspace | 786 tests passed: 768 outside net, 18 net tests with owned loopback servers |
| Formatting and Clippy | Workspace fmt and all-target Clippy with `-D warnings` passed |

The locked Python 3.11 environment and binding were rebuilt from this checkout
with Rust 1.99.1 and offline Cargo. Net tests ran separately after the sandbox
refused their local fake-server binds. Recordings are fresh worktree artifacts;
the retained corpus was read only. Full census/shadow shards remain with the
orchestrator. This checkpoint does not claim finalization-stage parity.
