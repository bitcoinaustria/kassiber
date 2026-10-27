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
semantics. Arrays preserve database order, including ties and unordered reads.
Unknown schema versions and operations are refused.

| Field | Raw facts and replaced database reads |
| --- | --- |
| `schema_version`, `operation` | `1`, `custody` |
| `as_of` | Explicit RFC3339 build instant; pair-dismissal expiry never reads a clock in the pure path |
| `profile` | Profile facts, including scope, policy and journal input version |
| `book_network` | Nullable gate facts: stored binding, all network observations including excluded rows, reference chain/network/payload facts from chain analysis observations, observer instances, observation provenance, policy epochs and UTXOs; epoch private-material/source **presence** only. Replaces `require_book_accounting` and its inventory reads without copying private policy material |
| `wallets` | Database-order wallet id/label/kind/account refs and normalized configuration facts |
| `transactions` | Active `t.*` rows in `occurred_at, created_at, id` order, wallet/account facts and the joined `observation_*` provenance columns. No closed authority object is supplied as an input |
| `rate_rows` | Raw rate/value/exact/amount columns in the original descending occurrence/creation order, without an added id tie-break. Replaces latest-rate SQL, not its decimal computation |
| `owned_index` | Null when no active row has provenance. Otherwise script/outpoint/txid maps, including chain/network scoped maps, restricted to references in active rows' stored graphs. Match lists keep insertion order. Ownership policy derivation and its inventory/history/checkpoint reads stay at loading; descriptors, xpubs and private material do not cross the seam |
| `ownership_warnings` | Non-fatal index-loader diagnostics |
| `components` | Authored-active headers **as stored**, ordered legs, allocations, economic terms and evidence commitments. No materialized state, validation, inferred allocations or blockers |
| `anchors` | Map from every transaction/anchor id referenced by authored legs to its joined transaction facts, including excluded anchors; explicit null for missing anchors |
| `anchor_wallets` | Wallet scope facts for all named leg wallets, including wallets outside the active row population |
| `revision_headers` | Stored revision-link targets, including targets outside this profile; absent target ids remain on their referring headers |
| `receipt_sources` | Finite `SourceScript` facts from `build_profile_source_index`, loaded only when a Bitcoin event has receipts in multiple wallets. Replaces wallet configuration, UTXO, descriptor-target and freshness-checkpoint reads; canonical owner recommendations and reconciled amounts are recomputed |
| `channel_records` | Database-order channel records with wallet configuration facts and backend chain/network, before lifecycle roles or transfer pairs are decided |
| `loan_legs` | Database-order nondeleted transaction ids and roles |
| `evidence_snapshots` | Stored author-local component activation snapshots, ordered by subject/quantity/detail hashes |
| `migration_issues` | Unresolved stored migration issues, ordered by creation/id; quarantines are recomputed |
| `dismissed_gap_fingerprints` | Unambiguous latest authored dismissal fingerprints, preserving the existing concurrent-review rule |
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
Owner-book recordings stay local and ephemeral. The recorder checks for wallet
key material before writing. `KASSIBER_TEST_CUSTODY_SNAPSHOT_VERIFY=1` replays the
request in a fresh process under another hash seed, with no database image.

`KASSIBER_TEST_CUSTODY_ENGINE=shadow|census` is installed only by test plumbing,
including Python child processes. Stage differences and binding failures are
recorded before raising, so catching an assertion cannot make the session pass.
Unported stages are listed explicitly. Product code reads neither switch and
never records or logs snapshots on its own.
