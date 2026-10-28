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

Continue this interpreter stage in the following order:

1. Ownership, recorded fan-out and multi-source consolidation
   (`ownership_transfers.py`, 2,486 lines), using existing graph/scope helpers.
   Their producers and the raw-snapshot generator/comparison slice have not
   been started at this commit boundary; the generic claim-boundary fan-out
   revalidator is already available.
2. Raw-observation holds and privacy boundaries (interpreter lines 967–1161,
   `privacy_hops.py`, 137 lines): conflicts, confirmation precedence, fan-out,
   unscoped review and dependency closure.
3. Channel lifecycle and loan roles (`lightning/channel_lifecycle.py`, the
   journal's `channel_context_from_records`, and interpreter role branches).
4. Native HTLC transitions (`custody_native_transitions.py`, 302 lines), with
   `transfer_matching.py` (1,751 lines) and `htlc_parser.py` (352 lines): scoped
   populations, exact proofs, refunds, dismissals, policy and fee timing. The
   shared compiler's proof validator is ported; proof production is not.
5. Native component reconciliation (`custody_native_reconciliation.py`,
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

## Shared JSON feature contract

Cargo unifies custody's `serde_json/arbitrary_precision` and `float_roundtrip`
features into the ledger build. The former preserves unbounded JSON integers;
the latter makes float literal loading agree with CPython. This is an explicit
shared build contract, not an isolated custody dependency choice.
`crates/custody/scripts/check_ledger_features.py` builds the ledger example twice,
with and without those features, and compares all 145 stored ledger response
strings byte for byte. CI's Rust job and the quality gate require this check.
