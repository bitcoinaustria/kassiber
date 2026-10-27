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

Port the remaining slices in this order; sizes are approximate Python source
lines to inspect, not estimates of Rust output:

1. Component materialization and validation, including commitments, cross-route
   checks, revision/membership conflicts and anchor blockers: about 4,000 lines.
2. Interpreter compilation and its ownership, transfer-matching, Lightning and
   native-transition helpers: about 8,000–10,000 lines, best split by interpreter.
3. Gap search over the supplied worklist: about 1,900 lines.
4. Quantity arbitration, postings and runtime composition: about 2,100 lines
   plus their small allocation and hold helpers.
5. Finalized tax projection: about 1,000 lines. Then join custody and `ledger`
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
