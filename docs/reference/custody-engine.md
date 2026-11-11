# Custody engine

> **Historical.** This page records the Python-to-Rust port. Python was removed
> under [plan 22](../plan/22-python-removal.md); the `kassiber/` paths and Python
> tests it cites exist only at the `archive/python-backend` tag.

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

The mutation catalog has 167 entries. Reviewer
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

The 10,000 generated books and configured
permutations pass, as do 195 recorded requests and their permutations. Four
daemon acceptance tests remain blocked by the unchanged sandbox owner-lock
restriction; full census/shadow verification belongs to the orchestrator.

Raw-observation holds, privacy boundaries and dependency closure are now ported.
Continue with:

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

The subsequent stored-mapping and arithmetic audit
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
and one receipt precision mutation to the retained 318.

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
This run stops at the
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
The six ownership scenarios retain their 250 successes per engine. Snapshot v1 remains raw-only, Python
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
match the full Python interpreter's claims and audits. All 681 catalog entries have named-test
kills. Python remains authoritative, snapshot v1 stays raw-only and
`interpretation` stays skipped.

## Shared JSON feature contract

Cargo unifies custody's `serde_json/arbitrary_precision` and `float_roundtrip`
features into the ledger build. The former preserves unbounded JSON integers;
the latter makes float literal loading agree with CPython. This is an explicit
shared build contract, not an isolated custody dependency choice.
`scripts/check-ledger-features.sh` builds the ledger example twice,
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
The catalog adds 114 safety entries to the retained 681. Snapshot v1 is unchanged
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

## Principal pricing, payouts and fee allocation checkpoint (R2f, partial)

The local `t3code/r2f-projection-2` checkpoint starts at `a178a48f` and adds
principal slice pricing, reviewed direct-payout admission/time/settlement and
price overrides, and stateful fee allocation under
`crates/custody/src/finalized_projection/`. These remain private comparison
primitives in the unpackaged `projection-primitives` example.
`finalized_projection` stays explicitly skipped. No Rust result supplies stored
state or tax inputs, and snapshot v1 remains raw-only.

Principal pricing preserves the original raw amount before replacing it with
the selected slice. An existing unit rate is unchanged; otherwise exact absolute
fiat value precedes the legacy float, and the original amount first divides by
100,000,000,000 in Kassiber's 32-digit half-even decimal context. The value then
divides by that rounded BTC denominator in the same context. Combining those
divisions changes rounding and is forbidden. Exact output uses fixed decimal
rendering, including trailing zeros and signed zero. There is no additional
quantize step in these Python helpers. All quantities and fee sums use BigInt.

Payout admission normalizes the raw amount and fee before parsing the reviewed
allocation, accepts only positive source-bounded principal, and retains the last
valid record per source. Settlement follows Python's amount/value/time/base-row
admission order. Legacy timestamp parsing reuses the CPython ISO instant parser
and separately reproduces the legacy and RFC2822 grammars, including their
Unicode-digit distinction, whitespace, offsets, fractional truncation and UTC
overflow errors. Missing times fall back to the observation without rewriting
its text. The settlement emits the inbound leg before the outbound sale.

Fee state indexes the complete supplied decision population, but does not
select rows. An observation's raw fee is allocated once. Reviewed fee slices
sum by atomic bundle or pair-claim prefix; the first move consumes a bundle's
fee. An ordinary reviewed component replaces its raw sibling fee, while native
losses and swap refunds remain additive. Subsequent standalone reviewed fee
calls are suppressed only after that bundle has been consumed. Callers retain
responsibility for selection, barriers and connected-path blocking.

### Frozen and generated evidence

The Python oracle calls the original projection helpers. For nested fee and
price-override logic it executes the original reference AST with explicit
already-selected operands; it does not reproduce the arithmetic in a second
Python implementation. Frozen vectors, generated calls and dictionary-order
permutations compare exact strings and serialized numeric representations,
including signed float zero. Null/empty primary results are not counted as
active; each scenario also has an independently compared positive companion.
Both engines assert the checked-in per-scenario activity manifest.

| New primitive family | Frozen scenarios | Generated batches per engine | Active results per engine |
| --- | ---: | ---: | ---: |
| Principal slice pricing | 27 | 540 | 2,120 |
| Reviewed payout admission | 16 | 320 | 1,100 |
| Payout timestamps | 24 | 480 | 1,700 |
| Payout settlement rows | 18 | 360 | 1,400 |
| Payout price overrides | 10 | 200 | 760 |
| Fee allocation | 15 | 300 | 2,300 |
| **New total** | **110** | **2,200** | **9,380** |
| Inherited row primitives | 42 | 840 | 3,180 |
| **Combined** | **152** | **3,040** | **12,560** |

The combined generator makes **6,080 calls per engine**, plus **6,080 dictionary
permutations**. Principal coordinates, source denominators, settlement amounts
and fee bundles cross machine ranges. Controls include both sides of half-even
ties, one-msat pricing, multiple fee slices/payouts, replacement versus additive
fees, malformed decimals, nonfinite values and simultaneous admission failures.
Additional local seeded differentials compare **10,180 timestamp cases** and
**5,000 money cases**, without differences. Their scripts and logs are under
`target/r2f2-verify/`.

### Python reference issue: nonfinite payout values

The reference settlement helper admits positive Infinity and emits nonfinite
fiat values and rates. This is a helper-boundary bug, not a claim that every
product admission path accepts such a review. Reproduce on this checkpoint:

```python
import math
from kassiber.core.custody_tax_projection import _direct_payout_settlement_rows
from tests.test_custody_quantity import _observation
source = _observation("source", "wallet", "outbound", 100)
inbound, outbound = _direct_payout_settlement_rows(
    source, {"payout_amount": 1, "payout_fiat_value": "Infinity"}, {}
)
assert inbound["fiat_rate_exact"] == "Infinity"
assert math.isinf(outbound["fiat_value"])
```

Python is unchanged. The frozen `payout_settlement_inf` vector preserves this
behavior for parity. Nonfinite floats use the existing `$float` fixture tags
only in the private primitive serializer, not in snapshot v1 or a product API.
A separate fix must review the reference admission contract before any such
change is ported.

### Remaining work

Selected MOVE/external/inbound composition, connected-path blocking, scoped
basis barriers, issues/quarantines and reviewed conversion metadata remain
unported. Their admission and ordering must agree before enabling finalized
projection. Journal fusion with `ledger` remains a separate step.

### Mutation catalog and verification

The catalog has **2,015 entries: all 1,955 inherited entries and 60 new
entries are killed**. The inherited recipes and frozen primitive entries remain
unchanged. Each counted entry has a passing unmutated named witness, a building
mutant and a failing named witness. The combined per-entry audit is
`target/r2f2-verify/mutation-summary.json`.

| New named witness | Killed mutations |
| --- | ---: |
| `projection_slice_pricing_vectors` | 22 |
| `projection_payout_admission_vectors` | 6 |
| `projection_payout_time_vectors` | 9 |
| `projection_payout_settlement_vectors` | 9 |
| `projection_payout_price_vectors` | 2 |
| `projection_fee_allocation_vectors` | 12 |
| **Total new** | **60** |

The full inherited catalog was rerun in disposable source/target batches.
A temporary-filesystem quota interruption left 430 valid kills; all interrupted
or nonbuilding attempts were excluded and the remaining 1,525 entries reran
under the worktree's `target/`. All 349 inherited named baseline filters and
six new filters passed. A redundant Unicode-normalization omission is equivalent
because the decimal library already normalizes digits; it is not counted.
A denominator-rounding survivor led to a stronger significant-digit boundary
fixture; the final recipe is killed. No survivor or failed build contributes a
kill. The final 60 new entries were rerun against the final source and fixtures.

| Verification | Result |
| --- | --- |
| Inherited raw generators | 24,000 books and 1,030 permutations; zero differences |
| Combined primitive generator | 152 active scenarios; 6,080 calls plus 6,080 dictionary permutations per engine; zero differences |
| Retained read-only replay | 159 requests and 159 permutations; zero differences or pre-request refusals |
| Fresh worktree replay | 168 requests and 168 permutations; zero differences or pre-request refusals |
| Focused census | 115 tests and 35 subtests; 112 comparisons per implemented stage; zero mismatches |
| Focused custody-shadow | 113 tests and 35 subtests; 110 comparisons per implemented stage and 110 frozen Python comparisons; zero mismatches |
| Finalized projection inventory | 0 compared; 112 skipped in census and 110 in shadow |
| Snapshot/shadow/mutation/port contracts | 40 tests and 14 subtests passed |
| Rust workspace | 792 tests passed: 774 outside net and 18 net tests with disposable owned loopback servers |
| Formatting and Clippy | Workspace fmt and all-target Clippy with `-D warnings` passed |

Shadow excludes the documented read-budget test whose second frozen build
changes its SQL count; census enforces it. Harness tests that intentionally
inject reference failures run separately without the comparison overlay. The
locked Python 3.11 binding was rebuilt from this checkout with Rust 1.99.1,
`MATURIN_NO_INSTALL_RUST=1` and offline Cargo. The retained corpus stayed read-only;
fresh recordings and verification artifacts are local under `target/`.
This checkpoint does not claim complete finalized-projection parity.

## Selected row composition checkpoint (R2f, partial)

The local `t3code/r2f-projection-3` checkpoint starts at `e589c9a3` and adds
`finalized_projection/composition.rs`. The unpackaged comparison example accepts
separate private composition operands, never snapshots or product requests.
`finalized_projection` remains skipped in the binding, census and custody-shadow.
Rust results never reach stored state; snapshot v1 remains raw-only.

Selected internal moves preserve fee allocation, row identities, same-asset
privacy review, cross-rail pair identities and source/target anchor linkage.
Non-events retain only known fees outside selected internal moves. External
confirmed/presumed slices preserve reviewed gift/lost/payment meaning, payout
pricing and policy-scoped carrying-value settlement. Receipt remainders index
**all** decision targets, including ineligible targets, and use exact ordered
BigInt intervals and a monotonic maximum cursor. Rowless virtual receipts never
become acquisitions. Known fee siblings may project at the barrier event;
fees behind a later barrier remain held. The final row contract still refuses
duplicate projected identities.

Connected blocking closes the undirected graph of selected internal moves
before row selection. A blocked suffix therefore withholds its prefix even
when that prefix is eligible. Anchor ids, not synthetic observation ids, define
the path. Quarantine cause/origin metadata is intentionally deferred.

Basis barriers use the first matching profile/network/exposure/unit pool;
Bitcoin and Liquid network aliases share exposure, while unrelated books,
networks and assets stay isolated. Event order starts with the minimum leg
time. Missing event order fails closed. Equal-time tie breakers cannot establish
chronology: only the barrier event itself retains its finalized siblings.
Decision eligibility remains the quantity stage's decision-slice selection.

### Comparison and activity accounting

`tests/custody_projection_composition.py` invokes the original Python compiler
for composition and executes the unchanged original nested closure/barrier AST
for focused helper vectors. It omits only issue/quarantine output and the
reviewed-conversion audit pass. Dictionary permutations preserve row lookup
precedence. No reference implementation is changed.

| New family | Frozen scenarios | Distinct generated inputs per engine | Dictionary permutations |
| --- | ---: | ---: | ---: |
| Selected row composition | 47 | 940 | 940 |
| Connected path blocking | 6 | 120 | 120 |
| Scoped basis barriers | 10 | 200 | 200 |
| **Total** | **63** | **1,260** | **1,260** |

Each scenario counts only its own distinct operands. No companion contributes
activity. Refusals count independently and compare error families explicitly:
140 generated refusals cover `TypeError`, `ValueError`, `KeyError` and
`InvalidOperation`. Every new scenario has 20 active distinct inputs. The named
`test_projection_composition_vacuous_is_rejected` proves an empty result fails
the activity check.

The inherited raw generators additionally compare **all five implemented stages**
for **24,000 books and 1,030 permutations**, without differences. Their separate
local activity audit counts **21,941 distinct primary inputs**, deduplicated
within each scenario, with explicit empty-quantity controls and no companion
contributions. The quantity review's source files are untouched. Per-engine
counts remain in `target/r2f3-verify/full-inherited-{0,1,2}.json`.

The inherited primitive generator is also corrected: companion calls no longer
contribute activity and repeated inputs count once. Its manifest explicitly
retains 13 zero-activity empty-success controls. The historical R2f/R2f2 activity
totals above included companions and repeated operands; they are superseded by
this accounting, rather than claimed as seed activity here. Refusal cases still
compare their own error families.

### Python reference issue: unstable dependency origins

The reference computes the same blocked path under different process hash seeds,
but may omit a root blocker from a dependent transaction's quarantine. Explicit
roots are not re-enqueued after acquiring additional roots, so the answer depends
on Python set iteration order. This concerns deferred quarantine metadata; the
selected-row blocking result is stable. Python is unchanged.

Save this as `target/r2f3-origin-repro.py` and run it with `PYTHONPATH=.` and
`PYTHONHASHSEED=1`, then `PYTHONHASHSEED=2` using the locked Python environment:

```python
import json
from copy import deepcopy
from kassiber.core.custody_tax_projection import compile_finalized_tax_projection
from tests.custody_projection_composition import seed, environment
v = seed()
d = deepcopy(v["decisions"][0])
d["source"]["observation_hash"] = v["observations"][1]["quantity_hash"]
d["target"]["observation_hash"] = v["observations"][2]["quantity_hash"]
v["decisions"].append(d)
_, _, state = environment(v)
result = compile_finalized_tax_projection(
    v["profile"], list(v["rows"].values()), state,
    blocked_transaction_ids=["source", "target"],
)
print(next(json.loads(q["detail_json"])["blocked_by_transaction_ids"]
           for q in result.quarantines if q["transaction_id"] == "other"))
```

Seed 1 prints `['source', 'target']`; seed 2 prints `['target']`. The prior
nonfinite-payout reference issue remains documented at the preceding checkpoint.

### Remaining work

Issue/quarantine composition, including explaining barrier-held receipts and
connected dependency origins, and reviewed conversion metadata remain unported.
Only after their parity gate may finalized projection enter the stage inventory.
Journal fusion with `ledger` remains a separate step.

### Mutation witnesses and verification

The checked-in catalog adds 29 entries to the 2,015 inherited entries.
Comparison witnesses include `projection3-barrier-comparison` and
`projection3-inbound-gap`; minimum/maximum witnesses are
`projection3-event-minimum` and `projection3-inbound-max`; field-choice witnesses
include `projection3-neighbor-anchor`, `projection3-inbound-principal` and
`projection3-payout-out-id`. Each entry names a frozen Rust test, independently
of the generators.

| New named witness | Mutations |
| --- | ---: |
| `projection_selected_composition_vectors` | 18 |
| `projection_connected_path_vectors` | 3 |
| `projection_scoped_basis_vectors` | 8 |
| **Total** | **29** |

A reviewed-fee side-label mutant initially survived because its fixture's slice
was ineligible. The corrected fixture selects that exact reviewed fee and kills
the mutant. A separate baseline failure came from reusing a Cargo target across
fresh scratch trees while preserving old source mtimes: a cached mutant appeared
fresh. Those failed baselines and the survivor contribute no kills. The final
new-catalog run clears custody artifacts first; inherited batches each own a
fresh target. Verification artifacts remain local under `target/r2f3-verify/`.

All **2,044 entries have valid named-test kills**, including the complete
**2,015-entry inherited catalog** and **29 new entries**. Each counted kill has
passing unmutated baselines, a building mutant and failing named tests. Initial
inherited batches retained their valid per-entry kills; eight final-source
replacement batches covered every remaining entry, and redundant original
runners were stopped after coverage was checked. All eight replacement batches
and the final new-catalog run exit successfully. The complete per-entry audit is
`target/r2f3-verify/mutation-summary.json`; no interrupted attempt, failed baseline,
survivor or build error contributes a kill.

| Verification | Result |
| --- | --- |
| Inherited raw generators | All five implemented stages: 24,000 books and 1,030 permutations; zero differences |
| New composition generator | 63 scenarios; 1,260 distinct inputs and 1,260 permutations per engine; zero differences; 140 refusals compared |
| Corrected inherited primitive generator | 152 scenarios; 882 distinct inputs and 882 permutations per engine; no companion contributions |
| Retained read-only replay | 159 requests and 159 permutations; zero differences or pre-request refusals |
| Fresh recording integrity/replay | 53 snapshots; 53 requests and 53 permutations; zero integrity problems, differences or pre-request refusals |
| Focused census | 40 tests; 53 comparisons per implemented stage; zero mismatches |
| Focused custody-shadow | 40 tests; 53 comparisons per implemented stage and 53 frozen Python comparisons; zero mismatches |
| Finalized projection inventory | 0 compared; 53 skipped in both focused lanes |
| Projection Python tests | 6 passed, including refusal and deliberately vacuous activity controls |
| Snapshot/shadow/mutation/inventory contracts | 54 passed and 14 subtests |
| Full mutation catalog | 2,044 killed: 2,015 inherited and 29 new; no survivors or build errors counted |
| Rust workspace | 795 tests passed: 777 outside net and 18 net tests with disposable owned loopback servers |
| Formatting/Clippy | Workspace fmt and all-target Clippy with `-D warnings` passed |

The locked Python 3.11 environment uses Rust 1.99.1,
`MATURIN_NO_INSTALL_RUST=1` and offline Cargo, including mutation children.
Networking crate tests use disposable owned loopback servers and require a
sandbox exception for their local binds. The retained replay corpus stays
read-only. No push or PR is part of this checkpoint.

## Reviewed conversion metadata checkpoint (R2f, partial)

The local `t3code/r2f-projection-4` checkpoint starts at `1526b1dc` and adds
`finalized_projection/conversions.rs`. Its private `conversions` operation
accepts already-finalized rows, existing cross-asset metadata and reviewed
conversion records. It does not read quarantine records or dependency origins.
`finalized_projection` remains skipped, Rust results never reach stored state,
and snapshot v1 remains raw-only.

The audit indexes rows by their original journal anchor, applies optional exact
BigInt amount filters before direction filters, and requires exactly one
outbound and inbound candidate. Missing candidates do not parse reviewed
amounts. Existing durable pair ids suppress later reviews before parsing their
amounts; a successful newly emitted pair suppresses subsequent duplicates.
Projected row ids, original transaction ids, reviewed asset overrides and the
Python falsey policy/kind defaults are preserved independently. Malformed row
amounts precede malformed reviewed amounts, outbound amount parsing precedes
inbound parsing, and required projected ids retain `KeyError` refusals.

### Comparison and activity accounting

`tests/custody_projection_conversions.py` executes the unchanged original
Python audit AST, starting after quarantine composition and ending before the
projection constructor. It neither reproduces the selection algorithm nor
executes the deferred origin propagation. The frozen Rust witness is
`projection_reviewed_conversion_vectors`.

| New family | Frozen scenarios | Distinct inputs per engine | Permutations | Active results per engine |
| --- | ---: | ---: | ---: | ---: |
| Reviewed conversion metadata | 24 | 480 | 480 | 320 |

Only each seed's own distinct operands contribute activity. No companion calls
are made. Eight explicitly listed empty-success controls remain zero; an
unlisted vacuous scenario fails `test_projection_conversion_vacuous_is_rejected`.
The 100 refusal results are included in the active count and separately compared:
40 `ValueError`, 40 `TypeError`, and 20 `KeyError`. Exact amounts cross machine
ranges; controls cover falsey values, numeric strings, Unicode integers, booleans,
float truncation, duplicate ids, both ambiguous leg directions and admission order.

### Remaining work and reference bugs

Issue/quarantine composition remains deferred until the Python dependency-origin
reference is fixed. Reviewed conversion metadata does not touch that boundary.
Only after the complete projection parity gate may the finalized stage enter
the inventories. Journal fusion remains separate. No new Python reference bug
was identified; the preceding unstable-origin and nonfinite-payout reproductions
remain applicable and Python production code is unchanged here.

### Mutation witnesses and verification

The new catalog has 19 recipes, all killed by
`projection_reviewed_conversion_vectors`. Comparison witnesses include
`projection4-amount-comparison` and the cardinality checks; minimum/maximum
witnesses include `projection4-amount-min`, `projection4-amount-max` and both
cardinality-minimum mutations. Field-choice witnesses independently exchange
anchors, amounts and asset fallbacks. Deduplication, policy and kind are also
mutated. Initial inbound-cardinality and duplicate-review survivors exposed
missing fixture distinctions: the final fixtures add inbound ambiguity and
matching anchors on the suppressed duplicate. The final 19-entry rerun passes;
no survivor, interrupted run or nonbuilding attempt contributes a kill.

| Verification | Result |
| --- | --- |
| Complete mutation catalog rerun | All 2,063 killed: 2,044 inherited and 19 new; zero survivors or build errors |
| Inherited raw generators | 24,000 books, 1,030 permutations, all five implemented stages; zero differences |
| Primary-input activity audit | 21,941 distinct inputs across 12 suites; no companion contributions |
| Combined generator/primitive pytest lane | 49 tests passed |
| New conversion comparison | 24 scenarios, 480 distinct inputs and 480 permutations per engine; zero differences |
| Additional seeded amount differential | 4,000 inputs, including 875 TypeError and 1,430 ValueError refusals; zero differences |
| Retained read-only replay | 159 requests and 159 permutations; zero differences or pre-request refusals |
| Fresh worktree replay | 17 requests and 17 permutations; zero differences or pre-request refusals |
| Focused census | 81 tests and 17 subtests; 17 comparisons per implemented stage; zero mismatches |
| Focused custody-shadow | 81 tests and 17 subtests; 17 comparisons per implemented stage and 17 frozen Python comparisons; zero mismatches |
| Finalized projection inventory | 0 compared; 17 skipped in each focused lane |
| Snapshot/shadow/mutation/inventory contracts | 55 tests and 14 subtests passed |
| Final fixture/inventory check | 11 tests passed |
| Rust workspace | 796 tests passed: 778 outside net, 18 isolated net tests |
| Formatting and Clippy | Workspace fmt and all-target Clippy with `-D warnings` passed |

The locked Python 3.11 environment and binding were rebuilt with Rust 1.99.1,
`MATURIN_NO_INSTALL_RUST=1` and offline Cargo. Network tests used disposable
owned loopback servers after the sandbox refused their binds. Fresh recordings
and verification artifacts stay under `target/r2f4-verify/`; the retained corpus
was read only. `origin-repro.py` there reruns the preceding documented origin
bug: hash seed 1 yields `['source', 'target']`, seed 2 yields `['target']`.
This checkpoint does not claim complete finalized-projection parity.

The complete inherited catalog was rerun in eight independent disposable
source/target batches, alongside the final new-entry batch. A local adapter
retains the shared scratch-tree checker's baseline, build, failure and restoration
checks, restricting Cargo builds and test filters to the targets containing each
named witness; debug information is disabled to reduce linking cost. All 359
named filters have passing unmutated baselines. Earlier interrupted whole-target
trials are excluded. The final per-entry audit is
`target/r2f4-verify/mutation-summary.json`; batch logs, target mapping, runner
adapter and audit script are retained beside it. Every final catalog id has
exactly one audited kill, with no missing or duplicate entries.

## Projection restack on reviewed quantity (R2f, partial)

Local branch `t3code/r2f-projection-5` starts at `fc63c1e0`. This checkpoint
completes the requested restack and inherited verification only. The complete
issue/quarantine compiler, barrier-held receipt explanations and dependency
origins still need a Rust port. `finalized_projection` remains skipped in PyO3,
census, custody-shadow and replay. Python owns production accounting and
persistence; snapshot v1 remains raw-only. Journal fusion follows complete
projection parity.

| Requested commit | Restacked commit | Scope |
| --- | --- | --- |
| `a178a48f` | `fc30eeb8` | Finalized row primitives |
| `e589c9a3` | `aa44ab11` | Pricing, payouts and fees |
| `1526b1dc` | `6ba5f6bf` | Selected rows and scoped barriers |
| `ddeec82f` | `dfb72b16` | Reviewed conversion metadata |
| `d73a348b` | `6300605e` | Conversion checkpoint evidence |
| `117eb35d` | `077d9f28` | Shipped Python finite-pricing fix |
| `ca9a672f` | `c80ee752` | Shipped Python fixed-point origin fix |

Only the first cherry-pick conflicted. Its mutation-catalog and custody-engine
reference conflicts were additive: both the quantity-review additions and the
projection additions were retained, with valid JSON object boundaries. The
remaining six commits applied cleanly. The quantity refusal harness, own-seed
activity rules, distinct compiler/runtime operands and mutation corrections from
`fc63c1e0` remain intact.

Rust now admits all four base-row price fields before chain evidence or quantity
parsing, rejects malformed/nonfinite payout values with Python's `AppError`
family, and checks derived rates before timestamps or base-row construction.
Positive-amount and empty-value guards retain Python's earlier precedence.
Scientific notation is used only for the mandatory legacy-float finiteness
check, avoiding fixed-format padding for extreme exponents; amounts and decimal
values retain their exact representations. Frozen `payout_settlement_inf` now
expects `AppError`. Nonfinite negative payouts no longer count as empty controls.
The fixtures also pin finite legacy-float overflow, derived-rate overflow,
price-before-amount errors and all four raw pricing fields, including very small
finite decimals.

Each projection seed owns its activity. Explicit empty-success controls remain
separate; refusals count only after comparison. Primitive engine evaluations are
deduplicated across labels as well as repeated variants. The named
`test_projection_duplicate_operands_evaluated_once` pins that rule;
`test_projection_primitive_vacuous_is_rejected`,
`test_projection_composition_vacuous_is_rejected`, and
`test_projection_conversion_vacuous_is_rejected` reject empty activity claims.
The quantity generator's independent-input and vacuity regressions also remain.

No new Python reference bug was found. The old origin reproduction, copied to
`target/r2f5-verify/origin-repro.py`, now prints `['source', 'target']` under
both `PYTHONHASHSEED=1` and `PYTHONHASHSEED=2`, using the fixed Python. Its
Rust-side completeness/sorting witness belongs to the deferred composition port.
The formerly accepted nonfinite-price reproduction is now a matched refusal.

| Verification | Result |
| --- | --- |
| Inherited generator/primitive pytest lane | 62 tests passed |
| Earlier five stages, inherited raw suites | 24,000 books, 1,030 permutations; zero differences; 21,941 distinct primary inputs across the 12 suites |
| Primitive projection generator | 173 scenarios, 1,153 distinct inputs per engine; zero differences; 358 distinct refusals compared (265 AppError, 50 TypeError, 41 ValueError, 2 OverflowError) |
| Selected-row composition generator | 1,260 distinct inputs and permutations per engine; zero differences; 140 refusals compared |
| Reviewed conversion generator | 480 distinct inputs and permutations per engine; zero differences; 100 refusals compared |
| Activity falsification | Direct vacuous primitive probe exits 1; named primitive/composition/conversion regressions pass |
| Retained read-only replay | 159 requests and 159 permutations; zero differences or pre-request refusals |
| Focused census | 146 tests; components 13 comparisons, remaining implemented stages 12 each; zero mismatches |
| Focused custody-shadow | 146 tests; same stage counts, plus 11 frozen Python comparisons; zero mismatches |
| Finalized projection inventory | 0 compared, 14 skipped in each focused lane, including refused/injected builds |
| Fresh synthetic recording integrity and replay | 25 snapshots, zero integrity problems; 25 requests and 25 permutations, zero differences or pre-request refusals |
| Projection / harness contracts | 11 final projection tests; 77 recorder/shadow/mutation/inventory contract tests and 14 subtests; all passed |
| Rust workspace | 817 tests passed, including disposable owned loopback-server tests |
| Formatting and Clippy | Workspace fmt and all-target Clippy with `-D warnings` passed |

Raw book totals above are corpus iterations, not distinct-input totals. The
primitive generator retains five shared operands across scenario labels, but
evaluates each only once per engine; scenario activity is not added together
and presented as an independent-input total. No companion contributes activity.

The locked Python 3.11 environment and binding use Rust 1.99.1,
`MATURIN_NO_INSTALL_RUST=1`, offline uv after setup, and offline Cargo. All fresh
recordings and verification artifacts stay under `target/r2f5-verify/`; the
retained corpus was read only. The full quality gate is not claimed and no
push or pull request is in scope. A manual second pass reviewed the restack,
pricing admission order, exact arithmetic boundary, fixtures and activity counts.

### Restacked mutation catalog

All **2,180 final recipes** have audited named-test kills: **2,173 inherited**
(including one adapted recipe) and **7 new**. All 384 named witness filters pass
unmutated. The full inherited catalog was rerun; no inherited kill was assumed.

| New pricing guard | Named witness |
| --- | --- |
| Base-row admission before quantity/chain parsing | `projection_slice_pricing_vectors` |
| Legacy-float finiteness and malformed-value error family | `projection_payout_settlement_vectors` |
| Settlement value admission and derived-rate overflow | `projection_payout_settlement_vectors` |
| Price-override derived-rate overflow | `projection_payout_price_vectors` |
| Principal derived-rate overflow | `projection_slice_pricing_vectors` |

The inherited `projection2-money-sign` recipe changed only a nonfinite decimal's
sign. Fixed Python rejects that value before its sign matters, so the old
recipe survives and is obsolete. It is replaced with a finite-price sign
mutation, killed by the same named `projection_slice_pricing_vectors` witness.
The old trial is explicitly excluded from kill accounting; its batch exits 1
for that obsolete recipe alone. The replacement has its own passing baseline,
building mutant and failing witness. All other batch entries passed their
original checker. The final per-entry audit verifies recipe equality against
the final catalog and rejects any missing, duplicate or unexpected failure.

The first full run was interrupted after the scientific-notation admission
correction; none of its trials count. The final eight-batch run plus the one
replacement recipe are audited in `target/r2f5-verify/mutation-summary.json`.
`audit-catalog.py`, the targeted Cargo adapter, baseline/build/kill logs and
recipe inputs remain alongside it. The final catalog has no surviving recipe
or build error. No equivalent, interrupted or failed-baseline trial is counted
as a kill.

## Issue/quarantine composition checkpoint (R2f, partial)

Local branch `t3code/r2f-projection-6` starts at `72325e38`, including the
restacked quantity corrections and both shipped Python fixes. The new private
`finalize` operation composes selected rows, reviewed conversion metadata and
quarantines. It is an unpackaged comparison operation, not snapshot admission.
`finalized_projection` remains skipped in PyO3, census, custody-shadow and
recorded replay. Rust results never reach stored state; snapshot v1 stays
raw-only. Python accounting is unchanged.

`finalized_projection/quarantines.rs` preserves interpreter record copies and
last-write precedence, dependency explanations, ranked quantity issues,
component-review instructions, exact sorted distinct issue/gap arrays, scoped
basis roots and missing payout prices. Barrier-held receipts gain an explanation
only when they book no row and have no existing explanation. Dependency roots
propagate to a fixed point through explicit roots as well as dependent nodes;
unrelated roots stay outside the explanation. JSON detail strings use the
CPython codec with Python's default separators and sorted keys. Eager diagnostic
construction before `setdefault` retention and quarantine sorting before the
projection constructor preserve refusal order.

### Independent comparison and activity

`tests/custody_projection_quarantines.py` calls the unchanged original Python
compiler. The Rust fixture witness is
`projection_quarantine_composition_vectors`;
`projection_dependency_origins_complete_and_sorted` independently asserts that
the old source/target/other path's dependent names exactly `['source', 'target']`.
The original `origin-repro.py`, retained under `target/r2f6-verify/`, prints the
same result under Python hash seeds 1 and 2. No new Python reference bug was found.

| New comparison | Count |
| --- | ---: |
| Frozen scenarios | 52 |
| Distinct raw private inputs per engine, excluding scenario labels | 1,040 |
| Row-order, root-order and issue-order permutations per engine | 1,040 |
| Active results per engine | 1,020 |
| Compared refusals | 180: 20 AppError, 120 KeyError, 40 ValueError |
| Differences | 0 |

Activity belongs only to each seed's own distinct input; there are no companion
calls. `dependency_absent_row` is an explicit empty-success parity control with
zero activity, rather than a claimed active scenario. Every other scenario
must be active. `test_projection_quarantine_vacuous_is_rejected` pins this
assertion, and a direct deliberately vacuous probe exits 1 with
`AssertionError: vacuous quarantine scenario`. Refusals are compared before
they contribute activity. The checked-in counts manifest freezes these totals.

### Mutation witnesses and verification

The new catalog adds 29 recipes. Comparison witnesses include explicit-root
exclusion, pool identity, issue multiplicity and receipt suppression.
`projection6-issue-minimum` changes minimum blocker selection to maximum.
Set/ordering witnesses reverse origins and gap ids, replace aggregate sets with
vectors and invert interpreter priority. Field-choice witnesses exchange
workspace/profile ids, issue ids/reasons, quantity hashes/anchors and payout ids.
Separate witnesses cover fixed-point root propagation, eager `setdefault`
admission and blocked inbound component/bundle anchors. Every recipe names a
frozen Rust witness, independently of the generated comparison.

| New named witness | Recipes |
| --- | ---: |
| `projection_quarantine_composition_vectors` | 26 |
| `projection_dependency_origins_complete_and_sorted` | 3 |
| **Total** | **29** |

All **2,209 final recipes** have audited named-test kills: the complete
**2,180-entry inherited catalog** and **29 additions**, with **386 named filters**
passing unmutated. Eight independent disposable source/target batches reran the
inherited catalog. After the final admission-order correction, a separate run
reran all 152 inherited projection recipes against the final source; those kills
replace their earlier attempts in the per-entry audit. The final new-catalog run
killed all 29 additions. No failed baseline, nonbuilding mutant, survivor or
interrupted attempt contributes a kill. The shared checker's targeted Cargo
adapter, recipes, logs and exact per-entry audit remain in
`target/r2f6-verify/mutation-summary.json`.

| Verification | Result |
| --- | --- |
| Earlier five stages, inherited raw generators | 24,000 books and 1,030 permutations; zero differences; 21,941 distinct primary inputs across 12 suites |
| Retained read-only replay | 159 requests and 159 permutations; zero differences or pre-request refusals |
| Fresh synthetic recording integrity/replay | 21 snapshots; zero integrity problems; 21 requests and 21 permutations, zero differences or pre-request refusals |
| Projection comparison pytest lane | 14 passed |
| Inherited quantity primitive lane | 16 passed |
| Snapshot/shadow/mutation/inventory contracts | 66 passed and 14 subtests |
| Additional raw refusal/vacuity/catalog contracts | 5 passed and 14 subtests |
| Focused census | 78 passed; components 9 comparisons, remaining implemented stages 8 each; zero mismatches |
| Focused custody-shadow | 78 passed; same stage counts plus 7 frozen Python comparisons; zero mismatches |
| Finalized projection inventory | 0 compared, 10 skipped in each focused lane, including one explicitly injected build |
| Complete mutation audit | 2,209 killed: 2,180 inherited and 29 new; no survivors or build errors |
| Required preflight shard | 57 passed, including complete reviewed test inventory and source/wire contracts |
| Rust workspace | 819 passed |
| Formatting and Clippy | Workspace fmt and all-target Clippy with `-D warnings` passed |

The locked Python 3.11 environment uses Rust 1.99.1,
`MATURIN_NO_INSTALL_RUST=1` and offline Cargo. The first workspace attempt hit
four sandbox-denied networking-test binds; the complete rerun with disposable
owned loopback servers permitted passed. Fresh recordings and all verification
artifacts remain under this worktree's `target/r2f6-verify/`; the retained replay
corpus was read only. No full quality gate, push or PR is claimed. A manual
second pass reviewed diagnostic precedence, fixed-point closure, exact JSON,
receipt suppression, refusal order and activity accounting.

The inventory was refreshed using `uv run --locked python -m tests.rust_port.classify_tests update`, and all quantity/projection entries were reviewed against their existing
oracle dispositions. The refresh is a separate small local commit. Preflight
also exposed inherited `error_codes.json` drift: ignoring source locations, its
only changes are the two existing `custody_pricing.py` validation sites from the
shipped Python fix. The reviewed fixture refresh is separate from the handwritten
port; no Python source changed. The final command
`uv run --locked python scripts/python_test_shards.py run preflight --ci` passes.
Preflight is included in this checkpoint's verification.

### Exactly what remains

1. Add a lossless raw-stage adapter from the already computed Rust stages and
   snapshot facts into this compiler. It must use the original confirmation-
   decorated row population, quantity observations/decisions/eligibility and
   pool roots, reviewed payouts/conversions, interpreter blocks/quarantines,
   and channel/interpreter non-event ids in Python's order. Preserve CPython
   special values and surrogate strings in copied interpreter diagnostics;
   converting them through serde's ordinary JSON values cannot establish parity.
2. Compare the **whole** finalized stage, including refusals, over the inherited
   raw generators, recorded requests and permutations. The private-operand
   comparison above does not prove that adapter or its admission order.
3. Only after that parity gate, enable the stage in PyO3 and all comparison
   inventories, proving nonzero finalized comparisons and zero mismatches in
   census/shadow and replay. Every stage must remain compared or explicitly
   skipped, including refused and intentionally injected builds.
4. Journal fusion with `ledger` follows complete projection parity.

## Lossless interpreter diagnostic seam (R2f, partial)

Local branch `t3code/r2f-projection-7` starts at `746cda8c`. This is a smaller
verified checkpoint toward item 1 above, **not** the raw-stage adapter or the
whole-stage parity gate. `finalized_projection` remains skipped in PyO3 and all
comparison inventories. Snapshot v1 remains raw-only; Python accounting and
stored state are unchanged. Journal fusion remains outside this change.

The private `compose_lossless` compiler accepts interpreter diagnostics as
`kassiber_pyjson::Value` records independently of its ordinary typed operands.
Copies, last-write retention, grouping keys and final sorting retain Python
strings and values throughout. Generated diagnostics join that same population
without narrowing the interpreter copies through serde. In particular, distinct
lone surrogate identifiers cannot collapse into replacement characters.
The existing serde-only fixture operation remains available for its ordinary
JSON operands; it is not the lossless adapter interface.

The unpackaged `projection-lossless` example separates the operand JSON string
from the CPython-decoded diagnostic population. Its independent Python test
calls the unchanged full reference compiler and applies the existing comparison
projection to both outputs, including explicit nonfinite float tags.

| New comparison | Result |
| --- | ---: |
| Distinct primary private inputs | 6 |
| Additional distinct diagnostic-order permutations | 2 |
| Successful comparisons | 7 |
| Compared refusals | 1 KeyError |
| Differences | 0 |

Each input is evaluated once per engine; activity belongs to that input alone.
Every result has rows, quarantines or a compared refusal. The inherited
`test_projection_quarantine_vacuous_is_rejected` still proves that an empty
result cannot satisfy activity. These counts are private-operand comparisons,
not raw-book activity or whole-stage admission evidence.

| New mutation | Named Rust witness |
| --- | --- |
| `projection7-diagnostic-order` | `projection_lossless_order_and_last_write` |
| `projection7-diagnostic-field` | `projection_lossless_order_and_last_write` |
| `projection7-diagnostic-special` | `projection_lossless_special_values` |
| `projection7-diagnostic-surrogate` | `projection_lossless_surrogate_keys_and_values` |

The first surrogate-key mutant survived a single-record witness because the
copied record still contained the original identifier. That attempt is not a
kill. The final witness contains two distinct surrogate identifiers which a
lossy conversion collapses; it kills the building mutant. The inherited
`projection6-receipt-existing` recipe was mechanically adapted to `PyStr`
membership and re-killed. Known Python direct-payout reference bugs are mirrored pending the separate
Python fix: R1 prices every external slice for a partial payout; R2 gives
partial carrying-value payouts duplicate row ids; R3 stores payout value
through a float.

| Verification for this checkpoint | Result |
| --- | --- |
| Projection Python comparison lane | 15 passed |
| New lossless Rust witnesses | 4 passed |
| Mutation rerun | 33 killed: 29 inherited issue/quarantine entries and 4 additions; 5 baseline named filters passed |
| Catalog applicability | All 2,213 recipes apply; not a full mutation rerun |
| Rust workspace | 823 passed |
| Workspace fmt and all-target Clippy `-D warnings` | Passed |
| Required inventory refresh and preflight | Refreshed; 57 passed |

The workspace initially encountered four sandbox-denied networking test binds;
the complete run with disposable loopback servers permitted passed. Offline
Rust 1.99.1 and locked Python 3.11 were used with
`MATURIN_NO_INSTALL_RUST=1`; `HOME` was not changed. Logs remain under
`target/r2f7-*.log`. A manual second pass reviewed lossless grouping, copy
precedence, final sorting and refusal retention. No full quality gate, raw
recording campaign, push or PR is claimed.

Remaining work is still items 1–3 above: connect the original confirmation-
decorated rows and all computed stages in Python admission order; compare the
whole stage over all inherited raw generators, retained recordings and fresh
census/shadow recordings; only then enable admission and prove nonzero stage
comparisons. The full inherited 2,209-entry mutation rerun is also outstanding;
catalog applicability is not a kill claim. This checkpoint must not be used to
claim completion of the adapter, the whole-stage gate or plan 21 R2f.

## Raw finalized-stage adapter checkpoint (R2f, partial)

Branch `t3code/r2f-projection-8`, based on `84b2181b`, adds the private raw-stage
adapter in `finalized_projection/runtime.rs`. It assembles the compiler from
snapshot rows and computed quantity/interpreter stages. Rows retain captured
confirmation provenance; pool identities shed only their comparison dataclass
tag. Reviewed payouts/conversions, eligibility, event order, barrier roots,
issues, blocks and both non-event populations remain distinct. Interpreter
quarantines pass directly to the lossless copier. Typed operands undergo an
exact CPython-codec round-trip check before crossing serde; a narrowing fails
closed. Projection AppError refusals map to the raw protocol's `validation`
code, without changing the private primitive's error-family contract.

The 16 raw-book Rust vectors and 55 adapter seam vectors compare against the
original Python compiler. The seam vectors independently isolate channel and
interpreter exclusions and duplicate-row last-write precedence. Their Python
fixture regeneration check passes. All 20 new adapter recipes have building
named-test kills (`projection_raw_adapter_vectors` and
`projection_adapter_input_families`). Initial survivors for the row-id and
non-event witnesses are excluded; strengthened witnesses kill the final
recipes. Detailed logs are under `target/r2f8-verify/`.

At this boundary the unpackaged `projection-stage` driver exercises the entire
stage from raw v1 requests. The retained read-only replay matches 159 requests
and 159 permutations with zero differences or refusals. The complete generator
campaign and inherited mutation rerun are still running; this boundary does not
claim their completion. PyO3, census, shadow and normal replay still skip the
stage. Python remains authoritative, Rust results do not reach stored state,
and snapshot v1 remains raw-only.

### Whole-stage parity gate

The unpackaged raw driver now matches the unchanged full Python entry point
across all twelve inherited raw generators: **24,000 books plus 1,030
permutations**, including projection refusals, with **zero differences**.
`tests/test_custody_projection_stage.py` uses the existing common comparison
projection and reports 13 passing tests, including fixture regeneration.
The read-only retained corpus independently matches **159 requests plus 159
permutations**, with zero differences and no refusals. Logs are
`target/r2f8-verify/raw-comparison.log` and `replay-projection.log`.
This establishes the whole-stage gate before enabling normal comparison
admission. Neither the binding nor census/shadow admission changes at this
boundary. The complete inherited mutation campaign remains outstanding.

## Journal fusion design (R2g; request-builder checkpoint)

This design-only checkpoint starts at `1b62f405`. It does not add a compared
stage or complete R2. The existing six-stage inventory remains unchanged;
the compared fusion stage is unimplemented, and none of the R2f counts below proves fusion parity.
The request-builder checkpoint below implements the first local boundary.

### Inputs, outputs and authority

The production join is `CustodyJournalBuilder.build()` in
`kassiber/core/custody_journal.py`. It passes the finalized projection, ordered
wallet references and direct-payout metadata to `TaxEngineLedgerInputs`.
`KassiberTaxEngine` in `core/engines/ledger.py` checks supported tax processing,
requires a `FinalizedTaxProjection`, and returns `empty_result(inputs)` when
there are no projected rows. That empty path skips configuration validation
and the engine call; it still follows the constructor's tax-support check.

For a non-empty projection, `ledger_request.build_request()` produces the
schema-2 `ledger` request. The PyO3 `kassiber_tax.ledger` function calls
`kassiber_tax_core::ledger`; `ledger_response.read_ledger()` renders the result.
The fusion port must build that same request from Rust custody's finalized
projection, ordered wallet references and raw profile facts, then invoke the
existing Rust tax engine. It must not reconstruct taxable rows from observations
or use Python's request as its own compiler input. Direct-payout records are
compatibility/report metadata, not an alternate source of tax rows.

The comparison output should expose the exact admitted request and the exact
ledger response, or the refusal and its admission stage. Empty projection is
an explicit no-engine outcome, not an empty schema-2 call. These outputs alone
do not prove the complete `build()` result: Python additionally renders journal
entries, enriches quarantines, accumulates holdings, compares quantity balances,
and builds ownership-review candidates/counts. Those joins need their own parity
evidence before claiming the joined `journal` operation complete.

### Admission and representation

Preserve the executed order in `ledger_request.build_request()`: configuration
checks; crossed-asset same-asset-move assertion; pool-scope validation; wallet,
row and pair invariants; asset-wide holds; then request construction. Within
construction, preserve field evaluation and row/pair order, including deferred
timestamp error objects versus immediately raised conversion failures. Compare
competing-invalid-input cases, not only one invalid field per case.

Keep amounts as BigInt msat throughout. Preserve canonical decimal coefficient,
exponent and signed zero, exact-pricing fallback order, CPython truthiness,
string conversion, timestamp semantics and exception families. Do not route
amounts through floats or bounded integers. Wallet order, projection order,
pair order, conflict-before-pending priority and sorted chain parents are
observable request facts. Unresolved custody stays excluded or quarantined;
fusion cannot infer a taxable event or basis to make the ledger accept it.

### Parity gate and R4 boundary

Use the unchanged Python request builder as an independent oracle over Python's
finalized projection. Compare it with the Rust-built request over Rust's
projection, then compare ledger responses from those independently built inputs.
Compare refusals as well as successes, including error family, structured
details and first-failure order. Keep empty controls distinct from active output.
Extend PyO3, census, custody-shadow and recorded replay together so fusion is
always compared or explicitly skipped, including decision-only builds, upstream
refusals and intentional injections. Snapshot v1 remains raw-only.

Generated activity must count only distinct inputs belonging to each seed;
companion output cannot satisfy activity. Add a deliberately vacuous probe that
fails the activity assertion. Add named mutation witnesses for ordering,
field-choice, omission and comparison operators, and rerun all 2,236 inherited
recipes. Rerun the 24,000-book/1,030-permutation generators and read-only retained
replay with every earlier stage still accounted for. Fresh artifacts belong
under this worktree's `target/`. No such fusion runs are claimed here.

Until R4 switches authority, Python remains authoritative for production
snapshot loading, pre-build repairs, pricing, custody selection, request
preparation, ledger-response rendering, balance/ownership joins, persistence,
freshness and worker orchestration. The tax engine remains authoritative for
tax computation as it is today. New Rust fusion results are comparison-only
and must never supply persisted journal state. R4 must verify the complete CLI
and daemon journal path, including these Python-owned joins and report-readiness
barriers, before replacing that authority.

### R2g reference admission order

`check_configuration()` used to collect assets in a set and evaluate
`_config_token()` inside `sorted()`'s generator, so with several invalid asset
names the reported one followed hash-table iteration order: with assets `BTC#`
and `ETH:`, `PYTHONHASHSEED=1` refused `ETH:` and seed `3` refused `BTC#`.
Python now validates the asset names in sorted order, so the first refusal is
the lexically smallest invalid asset under every seed, and the Rust builder can
derive it from its operands. `tests/test_custody_fusion_reference.py` pins the
refusal across eight hash seeds. The fix is local to this stack; the tax-engine
branches that own `ledger_request.py` need the same change when they are
restacked.

The 2,236-recipe mutation campaign, distinct-seed activity/vacuity checks,
24,000-book/1,030-permutation rerun, retained replay and all fusion harness
integration remain outstanding. Existing R2f results below are historical
evidence and are not claimed as fresh R2g runs.

| Reference-blocker checkpoint verification | Result |
| --- | --- |
| Competing-asset reference repro (before the fix) | 1 passed; two distinct structured refusals for identical operands |
| Workspace formatting and all-target Clippy `-D warnings` | Passed |
| Rust workspace tests | 827 passed; zero failed |
| Test inventory | Refreshed; new repro reviewed as an oracle |
| Required preflight shard | 57 passed, 56 subtests passed |
| Fusion compared | No; implementation and acceptance gates remain outstanding |

Checks used Python 3.11, Rust 1.99.1, locked dependencies and offline Cargo.
The first workspace run hit four sandbox-denied loopback binds; the complete
permitted rerun passed with disposable local test servers. Fresh verification
artifacts are under `target/r2g-fusion-verify/`. No retained corpus was changed,
and no push, PR or full quality-gate result is claimed.

## Rust journal request builder checkpoint (R2g step 1)

Branch `t3code/r2g-fusion-4`, based on `76375c27`, adds the comparison-only
`ledger_request::build_request` compiler. Its operands are a finalized Rust
projection, ordered wallet-reference key/value pairs and raw profile facts.
It preserves configuration, crossed-asset assertion, pool scope, invariants,
asset holds and construction admission order. Integer msat remain arbitrary
precision; exact decimal coefficient/exponent and signed zero are retained.
Empty projections produce an explicit no-engine outcome. As with Python's
request builder, the constructor tax-support gate belongs to the caller.
The compiler never invokes an engine or supplies persisted journal state.

The unpackaged `ledger-request` probe accepts either explicit compiler
operands or a raw snapshot plus ordered wallet references. In raw mode it
runs Rust custody to obtain its own finalized projection. The independent
Python oracle runs Python custody and the original Python request builder;
it does not supply its projection or request to Rust in that mode. This
probe does not extend snapshot v1 or normal comparison admission.

| Local builder verification | Result |
| --- | --- |
| New independent Python vectors | 145: 98 requests, 46 structured refusals, 1 empty control; zero differences |
| Existing ledger golden operands | 166; independently rebuild requests, including refusals; zero differences |
| Raw books, one seed for each inherited scenario | 515: 265 active requests, 243 empty controls, 3 builder refusals, 4 upstream refusals; zero differences |
| Named new mutation witnesses | Ordering, exact-rate field choice, omitted chain order, fee comparison, closed confirmation and cutoff diagnostics: all 6 building kills |
| Inherited mutation catalog | All 2,236 recipes still apply; not rerun at this checkpoint |
| Workspace Rust tests | 832 passed |
| Workspace fmt and all-target Clippy `-D warnings` | Passed |
| Python builder/reference tests | 5 passed, including the vacuous-probe rejection |
| Test inventory and preflight | Updated and reviewed; required preflight passed |

The raw probe counts activity only for distinct inputs owned by the current
seed. A deliberately vacuous probe fails even when an unrelated companion
produces active output; repeated copies of an input count once. Empty controls
and upstream refusals do not contribute activity. These are bounded builder
checks, not the full 24,000-book/1,030-permutation acceptance rerun.

Manual review corrected prototype cutoff error-object differences and
preserved captured confirmation provenance: a captured observation for another
physical scope must not cause recapture from enriched rows. No new Python
reference bug was found; Python production source is unchanged.

This checkpoint stops at step 1. Ledger responses are not compared. PyO3,
census, custody-shadow and recorded replay retain their previous six-stage
inventory and do not yet compare fusion. Steps 2 and 3 still require complete
competing-invalid-input/refusal parity, independent ledger-response parity,
the full inherited mutation rerun, the 24,000-book/1,030-permutation rerun,
read-only retained replay, and explicit fusion comparison/skip accounting in
every harness path. Python's post-ledger joins remain R4. Fresh logs are under
`target/r2g-fusion-verify/`; no push, PR or full quality gate is claimed.

## Complete finalized projection runtime (R2f)

The whole-stage gate above precedes normal admission. The binding now compares
`finalized_projection`, and census, custody-shadow and recorded replay include
it. `custody(request, finalized=False)` explicitly skips it for decision-only
builds; the control is outside snapshot v1. Refusals identify their compared
stage and skip the unavailable outputs. Intentionally injected failures retain
the explicit all-stage skip inventory, including finalized-stage injections.
No Rust result reaches stored state. Python remains authoritative, snapshot v1
stays raw-only, and journal fusion with `ledger` remains separate work.

`custody(request, loaded=True)` reads the snapshot in the native loader's own
record encoding instead: a SQLite BLOB is the codec's `$kassiber_bytes` tag,
and a mapping whose sole key is a tag is escaped as `$kassiber_dict`. It runs
`run_loaded` and fusion as journal builds do, and answers in that encoding,
since a stage may carry the book's bytes. The strict v1 adapter refuses REAL
or TEXT quantities and cannot carry bytes, so custody-shadow and census route
exactly those books (a BLOB anywhere, or a non-integer transaction amount or
fee) through it. The native loader gate then accepts its probe's v1 encoding
refusal when Python's own `json.dumps` of the snapshot refuses with the same
boundaries.

Confirmation provenance is captured once before rates/channel admission, in
Python's order. `projection_capture_precedes_rates` pins a closed timestamp
which overflows UTC conversion alongside malformed rates; both engines raise
`OverflowError` first. Interpreter-blocked quantity issues still contribute
component anchor suppression before they are excluded from generic issue
explanations. A dedicated adapter witness preserves that distinction.
`projection_adapter_lossless_diagnostics` independently retains surrogate ids,
NaN and infinity through the actual adapter seam.

| Final comparison | Result |
| --- | --- |
| Twelve inherited raw suites, after enabling | 24,000 books + 1,030 permutations; 24,852 successes and 178 compared refusals; zero differences |
| Empty-success controls within those successes | 305; compared explicitly, not counted as active output |
| Retained read-only replay, enabled stage | 159 requests + 159 permutations; zero differences or pre-request refusals |
| Fresh census recordings, integrity/replay | 29 snapshots, zero integrity problems; 29 requests + 29 permutations, zero differences |
| Fresh shadow recordings, integrity/replay | 29 snapshots, zero integrity problems; 29 requests + 29 permutations, zero differences |
| Focused census | 107 passed; finalized projection 8 compared / 5 skipped, other stages 12 compared / 1 skipped; zero mismatches |
| Focused shadow | 107 passed; same stage counts, 11 frozen Python comparisons; zero mismatches |
| Explicit injected builds in each focused lane | 1, with every stage skipped |
| Refusal/injection and child-difference harness | 48 passed, including finalized-stage refusal and injection inventories |
| Private projection comparison lane | 15 passed |
| Rust workspace | 827 passed |
| Workspace fmt and all-target Clippy `-D warnings` | Passed |
| Inventory refresh and required preflight | Updated and reviewed; 57 passed |

All verification uses locked Python 3.11, Rust 1.99.1,
`MATURIN_NO_INSTALL_RUST=1` and offline Cargo. The first workspace run encountered
four sandbox-denied loopback binds; the complete permitted rerun passed using
disposable owned servers. `HOME` was not changed. Local verification artifacts
are under `target/r2f8-verify/`; the retained corpus was read only. No full
quality gate, push or PR is claimed. Known Python direct-payout reference bugs are mirrored pending the separate
Python fix: R1 prices every external slice for a partial payout; R2 gives
partial carrying-value payouts duplicate row ids; R3 stores payout value
through a float. No Python production
source changed.

The final enabled generator rerun uses the normal Rust entry point and evaluates
each raw input once per engine, without companion activity calls. The fixture
lane separately verifies 16 raw-book witnesses, 56 adapter seam witnesses and
the admission-order counterexample against the original Python compiler.
Manual review covered admission order, original row selection, pool identity,
interpreter copies, component anchor suppression and complete stage inventories.

### Complete mutation rerun

All **2,236 final recipes** have audited building named-test kills: the entire
**2,213-entry inherited catalog** and **23 additions**. Across the campaign,
**393 distinct named filters** passed unmutated. Eight disposable source/target
batches reran the inherited catalog at the adapter boundary. After enabling,
the two mechanically reshaped inherited recipes (`rates-before-components`
and `REC-OPERATION-01`) and all 23 additions were re-killed against the final
implementation. Their final trials replace the earlier attempts in the audit.
No inherited kill was merely carried forward from the previous handoff.

| New named witness | Final recipes |
| --- | ---: |
| `projection_adapter_input_families` | 18 |
| `projection_raw_adapter_vectors` | 3 |
| `projection_adapter_lossless_diagnostics` | 1 |
| `projection_capture_precedes_rates` | 1 |
| **Total** | **23** |

The recipes cover raw-row order and field choice, original confirmation
provenance, observations, decisions, eligibility, canonical event ordering,
pool barriers/roots, issues and component anchor suppression, reviewed payouts
and conversions, interpreter blocks and copied diagnostics, both non-event
families, profile fields, lossless diagnostic admission and refusal families.
The old row-id and non-event witness survivors are explicitly superseded by
building kills from strengthened witnesses; none counts as a kill. The final
per-entry audit checks exact recipe equality, named-test multiplicity, successful
baselines, complete catalog membership and replacement trials. It reports zero
survivors and zero build errors in
`target/r2f8-verify/mutation-summary.json`. The targeted Cargo adapter, original
batch recipes, checker logs and `audit-catalog.py` remain alongside it.


## R2f independent-review corrections

The finalized row contract treats mappings as records of named fields. Mapping
insertion order is not an accounting or fusion contract: Python constructs row
keys in insertion order, while Rust's serde maps sort them. Comparisons preserve
sequence order (rows, pairs, selected moves and diagnostics), key sets, values,
types and exact decimal text, but intentionally ignore mapping key order.
`test_projection_row_mapping_order_is_not_a_contract` verifies this distinction
on actual finalized rows. Fusion must read row fields by name and must not hash,
zip or otherwise interpret their insertion order; canonical hashing must use
its explicit canonical serializer.

The review payout generator uses the real `create_direct_swap_payout` writer
with disposable SQLite books. Its expected outputs are computed live. Partial
payout cases are labelled "reference bug, expected to change" for R1/R2, and
all writer values remain subject to R3. These are parity evidence, not frozen
assertions of correct payout accounting.

Rust refusals now include `error.stage`: the first uncompleted Python stage.
Pre-component admission errors identify `components`; raw protocol admission
identifies `snapshot`. Census, shadow, PyO3 replay and the raw projection driver
compare this stage together with the error code. No successful outputs are
fabricated for an atomic refusal.
## Performance checkpoint (R2 perf)

Local branch `t3code/r2-custody-perf` starts at `1b62f405`. It makes the Rust
custody engine faster without changing a response byte, and it changes no
Python code. Snapshot v1, the response schema and every stage's semantics are
unchanged.

### Measurement tools

- The opt-in `timing` cargo feature records `Instant` spans per stage and
  sub-step. Without the feature, every span is a zero-sized no-op. The PyO3
  binding never enables it.
- The unpackaged `custody-bench` example (`--features timing`) times one raw
  snapshot file per stage. `--corpus <requests.jsonl>` prints the SHA-256 of
  the full and the decision-only response for every request. Every
  optimization was checked against baseline hashes over 25,348 requests: all
  twelve generator suites with their permutations, plus the 159 retained
  replay requests and their permutations. It was also checked against
  generic, multi-wallet, barrier and merged-generator books of 2,000 and
  20,000 transactions.

### What changed

- **Quadratic scans became indexed lookups.** This covers quantity
  eligibility, observation aliases, reviewed component observations, native
  support context and status, component lineage, membership and anchors,
  hold grouping, and every first-seen scope grouping. Each index preserves
  the original iteration order and duplicate handling. Membership through
  JSON equality uses serde's injective encoding.
- **Fewer copies.** One shared JSON view of the snapshot rows replaces the
  per-interpreter copies. The interpreter context and pipeline borrow the
  canonical evidence and the rows. `json!` constructions that serialized
  large operands now move them. Exactness round trips are skipped for values
  that provably convert losslessly.
- **Memoization per operation.** Embedded raw/configuration JSON and on-chain
  scopes are memoized per operation and cleared when the operation ends. A
  row-address hint for scopes still compares all eight scope fields.
- **Gap search.** States are keyed by dense id ranks, which order exactly
  like the sorted ids. Amounts use `i128` when every principal fits `i64`
  and there are fewer than 2^20 legs, and `BigInt` otherwise.
- **Faster JSON paths.** The CPython loader copies plain strings and parses
  short integers directly. pyjson copies printable ASCII runs whole. The
  response is written directly rather than composed as a Python dict.
- **Concurrency.** An order-preserving scoped-thread map (`par.rs`, at most
  eight workers, only for at least 128 items) handles pure per-row work:
  event keys and groups, enriched rows, confirmation capture, gap admission,
  scopes, native observations and hold facts. Results are consumed in order,
  so the first refusal is unchanged, and a worker panic is resumed in order.
  Arrays of at least 1,024 items convert, check and encode in chunks. A
  book's stage texts are encoded concurrently from 256 transactions. A large
  book's values are freed on a detached thread after its response is built.
  The engine falls back to the calling thread whenever no thread can be
  spawned.

Only one commit touches `finalized_projection/composition.rs`, so it can be
restacked independently. The following mutation recipes were re-anchored to
equivalent mutants of the rewritten code, each in the commit that changed
its anchor:

- QRT-056 and R2-review-audit-runtime-258/349/667
- QR-018, QR-027 and QR-050, with the reviewed-393/424 audits
- native11-native-guard-11 and REC-OPERATION-01/02
- QN-133 to QN-137 and E-aggregate-leg-tie
- the fourteen projection8 input recipes and projection8-row-order
- GAP-120, R2c-pipeline-samourai-reservation, and R2c-safety-legacy-confirmed
  and -pending-explicit

### Results

These were measured through the binding on a quiet 48-core machine, best of
three runs at 2,000 transactions and one run at 20,000. Responses are
byte-identical.

| Book | Python full / decisions | Rust before | Rust after |
| --- | --- | --- | --- |
| Generic 2,000 | 0.76 / 0.74 s | 2.44 / 2.10 s | 0.59 / 0.46 s |
| Multi-wallet with Liquid and pegs, 2,000 | 1.09 / 1.04 s | 4.28 / 3.91 s | 0.85 / 0.78 s |
| Liquid barrier, 2,000 | 2.18 / 2.15 s | 4.53 / 4.24 s | 0.84 / 0.86 s |
| Merged generators, 2,111 | 2.57 / 2.53 s | 5.56 / 5.18 s | 1.54 / 1.54 s |
| Generic 20,000 | 5.2 / 5.0 s | 23.9 / 19.0 s | 4.7–5.1 / 4.1 s |
| Multi-wallet, 20,000 | 12.6 / 11.9 s | 138.9 / 134.3 s | 7.2 / 6.1 s |
| Merged generators, 21,132 (full) | 93 s | 322 s | 48 s |

The engine is now four to twenty times faster than before and ahead of
Python on every book. It is not ten times faster than Python. At 20,000
transactions the floor alone is about one second: parsing the 68 MB request
takes about 0.45 s, and the 270 MB response must still be written. After
that, the remaining cost is spread across serde values: map lookups, string
allocation and conversions between serde and Python values. The largest
remaining items at 20,000 generic transactions are:

- finalized composition, 0.75 s;
- quantity arbitration, 0.5 s;
- interpretation, 0.46 s;
- parse, 0.45 s;
- evidence, 0.4 s;
- gap search, 0.36 s.

The merged-generator stress book also keeps quadratic interpretation and
component materialization for thousands of rows that share one fixed
transaction id. Real books do not share a txid that way.

### Verification

- Every commit kept all 25,348 corpus hashes and all bench-book hashes
  identical, including decision-only responses.
- The full mutation catalog was run on the final commit: all 2,236 recipes
  are killed, and the kill set equals the catalog.
- Workspace fmt, clippy `-D warnings` and the workspace tests (829) pass.
  `classify_tests update` leaves the inventory unchanged, and preflight
  passes 57 tests.
- Retained replay matches 159 requests and 159 permutations with zero
  differences. Focused census passes 604 tests, and shadow passes 602.
- The custody stage/port modules pass, with three exceptions. The same three
  tests also fail at `1b62f405`, which is not changed here:
  - the gap and interpretation fanout generator tests;
  - `test_full_stage_vectors_wrappers_and_refusals`.

  They call the binding with `finalized=True` against interpretation- and
  gap-only references, so the finalized stage's `validation` refusal
  surfaces.

## Interpretation and gap kernel checkpoint (R4)

Local branch `t3code/custody-perf-interp` starts at `dd433e4e` and replays
the R4-1 and rejected R4-4 probe records as `92ecc0562` and `4c130249c`.
The [typed-stage probes](r4-journal-pipeline.md#r4-4-typed-compilation-probe-stopped)
showed that typing records alone did not remove the expensive kernels.
This pass preserves each stage's input and output representation, with no
new conversions at neighboring stages. Python keeps production authority.

### Kernel changes

- Pipeline preparation borrows unchanged row slices through consolidation,
  ownership and fanout derivation. Fee normalization clones only when an
  actual override exists. An override still clones the whole slice; this
  is not a complete sparse row model.
- Holds share decoded raw facts, transfer scopes and confirmation states.
  Raw facts decode in parallel; scope calculation stays on the caller so
  it reuses recorded detection's operation-local scope cache. Conservative
  borrowed JSON hints skip impossible scopes and graphless decoding only
  when the top-level transaction identity is invalid. Oversized integers,
  duplicate hint fields, nested payloads, explicit status and malformed JSON
  retain the full decoder and its refusal behavior.
- Empty pair derivations and quarantine populations skip unused indexes.
  Pair scopes cover only pair endpoints. Observation lookup borrows hash
  keys while preserving ordered alias output. Lightning provenance is
  decoded only for recognized payment-hash sources.
- Normal stage composition avoids building isolated diagnostic output;
  that diagnostic entry point retains all its fields. Released contexts
  clone lazily. Reconciliation indexes every claim-source anchor, including
  intermediate native-route rows, rather than copying every raw row.
- Gap admission normalizes once before ignored-id filtering, including rows
  ignored by both search populations. Both advisory and accounting search
  reuse those legs. A local borrowed worklist view avoids cloning maps;
  the only shared changes extract existing field access and raw decoding.
- The return beam skips deduplication for newly seen identity ranks. Repeated
  identities retain ordered replacement. States append using the original
  prefix length, and sorted rank keys extend incrementally. Small integer
  sorting borrows keys; arbitrary integers retain cached keys. Sorting and
  duplicate-ID behavior remain deterministic. Independent Python witnesses
  cover both amount paths and duplicate identities with different principal.

### CPU and wall measurements

The saved before/after release binaries alternate in **five pairs per book**.
Each table entry selects the minimum process CPU for that span and reports
the wall span from the same run. CPU uses `CLOCK_PROCESS_CPUTIME_ID`, including
worker-thread user and system time; whole-child CPU independently uses
`getrusage(RUSAGE_CHILDREN)`. Temporary instrumentation is absent from product
source. Both binaries use Rust 1.99.1, locked offline dependencies, the same
R4 snapshots and the same harness. The one-minute load at run endpoints is
**14.31–17.88**. CPU is the acceptance measure on this loaded machine.

| Book | Span | CPU before → after, s | Wall before → after, s | CPU speedup |
| --- | --- | ---: | ---: | ---: |
| Generic 2k | Interpretation | 0.077767 → 0.018858 | 0.047495 → 0.015756 | 4.124× |
| Generic 2k | Gaps | 0.110385 → 0.054654 | 0.092976 → 0.034154 | 2.020× |
| Multi-wallet 2k | Interpretation | 0.196935 → 0.058974 | 0.167470 → 0.053655 | 3.339× |
| Multi-wallet 2k | Gaps | 0.079975 → 0.038347 | 0.059959 → 0.019636 | 2.086× |
| Generic 20k | Interpretation | 0.764211 → 0.221165 | 0.463162 → 0.197656 | 3.455× |
| Generic 20k | Gaps | 0.619846 → 0.286424 | 0.456036 → 0.092145 | 2.164× |
| Multi-wallet 20k | Interpretation | 2.055276 → 0.674136 | 1.770385 → 0.630862 | 3.049× |
| Multi-wallet 20k | Gaps | 0.583284 → 0.276918 | 0.396418 → 0.089269 | 2.106× |

Interpretation meets the one-third CPU target and gaps meet the one-half
target at both sizes on both books. Independently selected whole-child CPU
and its associated wall time, including the other stages and the ledger:

| Book | Child CPU before → after, s | Child wall before → after, s |
| --- | ---: | ---: |
| Generic 2k | 1.616503 → 1.489383 | 0.905023 → 0.840125 |
| Multi-wallet 2k | 2.233190 → 2.030617 | 1.359048 → 1.203111 |
| Generic 20k | 16.055593 → 14.694648 | 8.683180 → 7.865738 |
| Multi-wallet 20k | 21.975216 → 19.564895 | 12.859271 → 10.752868 |

Per-span minima are not added to
claim a synthetic whole-pipeline time. These measurements do not establish
the broader R4 target of 10× Python custody or 4× end to end.

### Byte parity and activity

The independent Python/Rust campaign covers all twelve generators:
**24,000 books and 1,030 permutations**, with zero stage differences.
There are 178 matching finalized-stage refusals; decision-only comparison
also checks their earlier stages. Per-seed records count canonical distinct
inputs and active outcomes. A separate audit recomputes these counts,
including distinct inputs with interpretation claims and gap candidates.
A deliberately vacuous seed, whose only active output belongs to another
input, fails with `no distinct active input belongs to the seed`.

Saved unchanged-baseline and final binaries additionally compare literal
full-response, decision-response and stage byte hashes for all **25,030**
generated requests, **159 recorded requests plus 159 permutations**, **1,134
runnable frozen inputs** and all four benchmark snapshots. No hash changes.
The frozen sweep includes 115 matching whole-pipeline refusals. One additional
fixture operand, `custody_projection_adapter.json`'s
`issue_before_constructor_refusal`, is deliberately an isolated adapter input
without a component header `profile_id`. Both baseline and final panic when
it is incorrectly sent to the whole-pipeline API; its intended adapter entry
point is covered by the frozen workspace tests.

Evidence remains under `target/r4-perf/`: `wire-summary.json`, the paired
hash files, `campaign-seed-activity-audited.jsonl` and
`campaign-activity-audit.json`. The retained replay uses
`/home/tg/.cache/kassiber-verify/custody-recordings/`.

### Mutation anchors and verification scope

The catalog retains every one of the **2,292 inherited IDs** and adds nine
witnesses, for **2,301** recipes. This run selects all **391 recipes in touched
files**, including both minimal shared helper extractions. It does not claim
a fresh kill of the remaining 1,910 untouched recipes. Every selected recipe
must build and fail all its named filters, and every named filter must pass
unmutated. Compilation errors are not kills.

Moved recipes retain equivalent mutations:

| Recipe | Equivalent mutant at the new anchor |
| --- | --- |
| `I-shared-aliases` | Take only the first alias per event in the flattened iterator. |
| `R2c-graph-raw-hex-gate` | Remove the component-asset admission branch in the extracted decoded-object parser. |
| `R2c-safety-positive` | Admit zero principal through the shared sign predicate on both integer paths. |
| `R2c-safety-nested-status` | Disable the shared nested-payload predicate for capture and cached facts. |
| `R2c-safety-pending-state` | Select confirmed rather than unconfirmed scopes in the borrowed state filter. |
| `R2c-safety-holds-normalized` | Feed raw rather than normalized derivation rows to fanout holds. |
| `REC-SAFETY-03` | Insert rather than remove reconciled anchors through the lazy context accessor. |
| `GAP-116` | Omit authored exclusions through the extracted helper. |
| `GAP-120` | Skip initial admission for IDs ignored by both search populations. |
| `GAP-065` | Invert empty-state priority in the extracted comparator shared by both amount paths; recipe text remains unchanged. |

The first new duplicate-ID recipe survived the original whole-search hash
witness. That attempt is not counted as a kill. A stronger independently
computed Python return-group case pins replacement of repeated identities
with different principals, on both the small and arbitrary-integer paths.

All **391 selected recipes** have building named-test kills; the kill set
equals the selected catalog. All **127 named filters** pass unmutated.
The nine new recipes and all 382 inherited touched-file recipes are included.
The single scratch source/build tree is deleted after its summary is saved;
`mutation-results.jsonl` retains each exact recipe and named-filter result.

The core-accounting census exposed two pre-existing test-plumbing failures:
`test_split_payout_fails_closed` deliberately splits Python's intermediate
projection, while Rust receives the unchanged raw snapshot. Both saved
binaries succeed on those two raw inputs with identical stage/full/decision
bytes. The test now uses the existing exact-stage, exact-exception injection
hook only for its expected split-payout validation error. It still requires
the original compiler to raise that error. The correction only annotates the
expected test injection; census and shadow pass the complete 94-test module
with exactly two recorded intentional injections.

Shard verification uses the test socket guard with the product no-egress
override unset, and a disposable cache `TMPDIR` outside the checkout. The
tax-shadow session probes expect an external temporary test-module path;
putting that scratch inside the checkout produces relative-path assertion
failures. All five session probes pass with the external cache root.

| Final verification | Result |
| --- | --- |
| Core-accounting census | 2,033 passed, 75 skipped, 460 subtests; 260 custody snapshots; zero differences |
| Reports-contracts census | 2,475 passed, 10 skipped, 2,824 subtests; 209 custody snapshots; zero differences |
| Core-accounting custody-shadow | 681 passed, 40 skipped, 231 subtests; 252 frozen Python comparisons; zero differences |
| Reports-contracts custody-shadow | 1,875 passed, 10 skipped, 2 deselected, 146 subtests; 205 frozen Python comparisons; zero differences |
| Tax census in both custody modes | Core-accounting: 548 calls from 18 modules; reports-contracts: 332 calls from 32 modules |
| Rust workspace tests, including doctests | 1,496 passed; zero failed or ignored |
| Workspace formatting and all-target Clippy `-D warnings` | Passed |
| Guarded chain and net builds | 20 and 63 passed, respectively |
| `classify_tests update` and preflight | Inventory unchanged; 57 passed, 60 subtests |

The shadow runner's two existing query-budget deselections are the parameter
variants of `test_disjoint_reviewed_routes_limit_observation_reads`; census
owns and passes that single-build budget check. Other intentional skips remain
explicit in the logs. Default Rust workspace tests use the repository gate's
ordinary environment, without the Python test-guard variable. Arming that
variable on the non-observer build correctly terminated an initial attempt
with exit 78; that attempt is retained, not counted as a pass. Separate chain
and net runs enable their observer features with the test guard armed.

All final logs are under `target/r4-perf/`. Manual second review checked
admission and refusal order, intermediate claim-source anchors, alias order,
and repeated-identity beam replacement. The final evidence audit records
source and artifact hashes before local commit; build scratch is removed
after summaries are saved. No dependency or lockfile changes are required.

### Remaining cost

Interpretation still decodes confirmation facts and prepares derivation
inputs; actual normalization overrides still require whole-slice copies.
At 20k, the best final holds CPU is 0.099 s generic and 0.162 s multi-wallet;
multi-wallet preparation is 0.146 s, including 0.112 s of derivation.
Gap admission is 0.230/0.220 s and search is 0.048/0.049 s
(generic/multi-wallet), independently selected subspan minima. Admission
dominates these books; beam vectors remain a cost on candidate-heavy books.
Quantity, finalized projection, request fusion and ledger work dominate the
broader pipeline and belong to the parallel workers. No dependency, schema,
Python authority or external-I/O contract changes in this checkpoint.

## Private ledger-response checkpoint (R2g step 2, bounded)

Branch `t3code/r2g-fusion-5`, based on `580015d8`, extends the unpackaged
`ledger-request` probe with an explicit `fusion` control. Active outcomes expose
both the independently Rust-built request and the existing Rust ledger's complete
response. Empty projections remain explicit no-engine outcomes; upstream and
builder refusals bypass the ledger. The probe uses a development dependency on
the existing tax engine; production custody has no new runtime dependency.

The request's exact compact JSON text is also compared with Python's production
`ledger_request.dumps()`. This is necessary for refusal parity: a request with
an integer amount above the ledger's `u64` range reports the JSON column in its
`RequestError`. Sorting object keys changed that diagnostic even when the
request documents compared equal. `request_json()` retains Python's field order
and literal Unicode; it does not normalize engine errors or discard columns.
Five independently regenerated wire witnesses cover normal rows, oversized
amounts, typed timestamp errors, Unicode timestamp diagnostics and closed
confirmation provenance.

| Bounded ledger-response comparisons | Result |
| --- | --- |
| Independent builder vectors and inherited ledger golden operands | 311: 243 admitted requests, 67 builder refusals, 1 empty control; zero differences |
| Ledger results for those admitted requests | 213 successes, 30 exact refusals; zero differences |
| Ledger refusal stages | request 2, prepare 3, validate 3, compute_multi 15, compute 5, classify 2 |
| Raw books, one seed for each inherited scenario | 515: 265 active ledger successes, 243 empty controls, 3 builder refusals, 4 upstream refusals; zero differences |
| Activity | 265 distinct active inputs owned by their current seeds; companion output excluded; vacuous-probe rejection retained |

These are live comparisons against the unchanged Python builder and the same
existing ledger engine invoked separately on each engine's request. Direct
payout cases are labeled **reference bug, expected to change** when encountered;
none occurred in this bounded raw sample, and no new payout golden is added.
No new Python reference bug was found. Python source is unchanged.

This is a private probe boundary, not normal fusion admission. PyO3, census,
custody-shadow and recorded replay still have their prior stage inventory and
do not compare fusion. Builder/upstream refusing-stage comparisons remain with
the separate harness work. Constructor tax-support admission remains a caller
responsibility. The 24,000-book/1,030-permutation campaign, 159-request retained
replay, fresh census/shadow recordings, all 2,236 inherited mutation reruns, and
new end-to-end fusion mutation witnesses remain outstanding. Rust fusion results
never reach persisted state; snapshot v1 remains raw-only. Fresh artifacts are
under `target/r2g5-verify/`. No push, PR or full quality gate is claimed.

| Checkpoint verification | Result |
| --- | --- |
| Python fusion/builder/reference comparisons | 7 passed; separate wire-fixture regeneration test passed |
| New wire-order/field-choice/omission/comparison mutation recipes | All 4 building named-test kills; successful unmutated baseline |
| Complete current mutation catalog applicability | All 2,246 entries apply; inherited kills are not claimed as rerun |
| Workspace formatting and all-target Clippy `-D warnings` | Passed |
| Rust workspace tests | 833 passed; zero failed |
| `classify_tests update` and required preflight shard | Refreshed; 57 passed, 56 subtests passed |

Verification used Rust 1.99.1, locked Python 3.11 and offline Cargo, without
changing `HOME`. The workspace rerun permits only the tests' disposable local
servers. An initial mutation baseline failed to link (linker bus error); it
counts as neither a kill nor a baseline. The complete four-recipe rerun used
bounded build concurrency and worktree-local scratch storage and passed.
Manual review checked exact wire order, empty-path engine exclusion, independent
request inputs, full response comparison and production authority boundaries.

## Fusion harness integration checkpoint (R2g step 3)

Local branch `t3code/r2g-fusion-6`, based on `e2be9b41`, adds comparison-only
`fusion` to the PyO3, census, custody-shadow and recorded-replay inventories.
The binding supplies the existing ledger function to custody's pure fusion
adapter, retaining custody's runtime dependency boundary. Rust builds from its
own finalized projection and ordered raw wallet facts; the independent Python
oracle builds from Python's projection. Comparisons include the request document,
exact request text and full ledger response, including ledger refusal stages.
Empty projections remain explicit no-engine outcomes. The standalone custody
operation explicitly skips fusion when no ledger callback was supplied.

Decision-only builds skip finalized projection and fusion. Atomic upstream
refusals identify the refusing custody stage; the harness compares that stage
and skips unavailable outputs. Same-code/wrong-stage responses fail comparison.
Exact intentional failure annotations skip all seven stages, including injected
fusion failures, and record their refusing stage. Fresh-process verification
regenerates fusion when present in the recording. These controls live outside
snapshot v1, which remains raw-only. No Rust result supplies production state.

| Integration verification | Result |
| --- | --- |
| Focused census | 58 passed; fusion 58 compared / 1 intentionally skipped; zero differences |
| Focused shadow | 58 passed; same stage counts; 57 frozen Python comparisons; zero differences |
| Refusal, injection, child-discrepancy and decision-only harness | 59 passed |
| Read-only retained replay | 159 requests + 159 permutations; zero differences |
| Fresh census recordings | 50 snapshots; zero integrity problems; 50 requests + 50 permutations; zero differences |
| Fresh shadow recordings | 50 snapshots; zero integrity problems; 50 requests + 50 permutations; zero differences |
| Rust workspace | 833 passed |
| Workspace formatting and all-target Clippy `-D warnings` | Passed |
| Inventory refresh and preflight | Updated; 57 passed, 56 subtests passed |

Verification uses Rust 1.99.1, locked Python 3.11, offline Cargo and
`MATURIN_NO_INSTALL_RUST=1`, without changing `HOME`. Fresh artifacts are under
`target/r2g6-verify/`. Initial `/tmp` SQLite I/O and sandbox quota failures were
superseded by successful runs using worktree-local temporary storage; the
workspace rerun permits its disposable loopback servers. No push, PR or full
quality gate is claimed. The full campaign is recorded separately below;
this integration checkpoint does not claim all inherited mutation kills.
Python production source is unchanged; partial-payout cases remain
**reference bug, expected to change** on restack.


### Fusion generated campaign and bounded mutation rerun

With the integration tree committed as `0907ca83`, the normal PyO3 custody
entry point compared every successful raw input across all seven stages,
including fusion.
Each engine computed its own projection and request. Upstream refusals compared
the code and refusing stage and explicitly left fusion unavailable. The campaign
script and machine-readable matrix are in `target/r2g6-verify/campaign.py` and
`campaign.json`; the audited aggregate is `summary.json` in that directory.

| Raw suite | Books | Permutations | Active requests | Empty controls | Builder refusals | Upstream refusals | Active distinct seed inputs |
| --- | ---: | ---: | ---: | ---: | ---: | ---: | ---: |
| evidence | 2,000 | 58 | 1,419 | 568 | 71 | 0 | 1,379 |
| components | 2,000 | 312 | 205 | 2,107 | 0 | 0 | 177 |
| samourai | 2,000 | 68 | 1,642 | 426 | 0 | 0 | 1,588 |
| recorded | 2,000 | 92 | 1,731 | 361 | 0 | 0 | 1,655 |
| fanout | 2,000 | 94 | 1,564 | 352 | 0 | 178 | 1,494 |
| graph | 2,000 | 16 | 2,016 | 0 | 0 | 0 | 2,000 |
| pipeline | 2,000 | 24 | 1,856 | 168 | 0 | 0 | 1,834 |
| safety | 2,000 | 78 | 797 | 1,281 | 0 | 0 | 767 |
| channels | 2,000 | 64 | 1,613 | 451 | 0 | 0 | 1,563 |
| native | 2,000 | 110 | 1,689 | 345 | 76 | 0 | 1,601 |
| reconciliation | 2,000 | 50 | 1,968 | 82 | 0 | 0 | 1,920 |
| gaps | 2,000 | 64 | 257 | 1,807 | 0 | 0 | 249 |
| **Total** | **24,000** | **1,030** | **16,757** | **7,948** | **147** | **178** | **16,227** |

All **25,030 input comparisons have zero differences**. Every admitted request
in this raw campaign produced a successful ledger response. The separate
311-vector comparison additionally verifies 30 exact ledger refusals across
request, prepare, validate, compute_multi, compute and classify. The 515-book
bounded raw probe and the vacuous-probe rejection were rerun successfully.
Activity is accumulated per current seed using distinct owned input documents;
permutations and companion output cannot supply the seed's activity. Every
suite has non-zero activity, and a deliberately vacuous probe fails.

| Mutation evidence | Fresh result |
| --- | --- |
| Full catalog applicability | All 2,246 recipes apply |
| All 10 fusion recipes | 10 building named-test kills; 3 unmutated baseline filters passed |
| 23 inherited `projection8-*` recipes | 23 building named-test kills |
| Mechanically updated `REC-OPERATION-04` and `REC-OPERATION-05` | 2 building named-test kills |
| Baselines for the 25 inherited trials | 5 named filters passed unmutated |
| Remaining inherited recipes | **2,211 not rerun; no fresh kill claim** |

The two operation recipes changed only to retain the added custody refusing
stage in their source anchors and replacements. The audit checks exact recipe
equality against the final catalog, one kill per selected recipe, and successful
baselines. No survivor or build failure occurred in these 35 final trials.
This is a bounded mutation rerun, **not completion of the requested full
2,236-entry inherited campaign**. Finish those 2,211 trials before declaring
that acceptance gate complete.

The integration verification table above remains applicable: Rust workspace,
formatting and Clippy passed. Immediately before the final evidence commit,
`classify_tests update` and preflight were rerun: 57 passed and 56 subtests
passed. Python production source
remains unchanged. No new Python bug was found. No direct-payout records were
encountered in the generated raw campaign; partial-payout cases retain the
label **reference bug, expected to change** and require revalidation after the
pending Python fix is restacked. No partial-payout golden was introduced.
The retained corpus was read only; no push or PR was created.

### R6 integration reference alignment

Partial direct payouts apply authored terms only to the reviewed decision and matching component. Residual slices retain ordinary pricing and carrying rows occur once. Real-writer payout parity now asserts the corrected Python reference, including exact persisted decimal terms.

Fusion appends its comparison result to the existing lossless stage values, preserving lone surrogate code points in diagnostics. Decision-only responses explicitly skip both finalized projection and fusion.
