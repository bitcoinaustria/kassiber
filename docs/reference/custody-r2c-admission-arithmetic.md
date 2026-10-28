# R2c stored-mapping and graph-arithmetic audit checkpoint

Branch `t3code/r2c-interpretation-6` starts at `8a70ae74`. Python remains
authoritative, snapshot schema 1 remains raw-facts-only, and `interpretation`
remains in `skipped_stages`. These audits precede ownership production.

## Stored-mapping admission

The previous serde adapter erased nonfinite stored JSON scalars before graph
parsing. `graph/stored.rs` now keeps `kassiber-pyjson` values through mapping
admission, graph merging and profile sibling merging. Only the partial wire
projection uses the existing test harness's `{"$float": ...}` encoding. A
literal object with that spelling is never decoded as a scalar. Producers must
consume typed facts when making scalar-sensitive decisions.

Findings and fixes:

- Infinite input/output indices raise `OverflowError`; NaN yields the caught
  `ValueError` and the corresponding absent/positional index. Invalid or
  coinbase input txids skip vout conversion first. Direct mappings precede
  nested mappings, ownership mappings and raw-hex decoding.
- Copied nonfinite script, txid and component values remain present. Asset,
  role, chain and network strings use CPython string conversion and case helpers.
- Int/float equality compares exact values without rounding a large integer to
  f64. Scalar NaN remains unequal; list/dict equality retains the identity
  shortcut for CPython JSON's shared NaN constant. Profile merging preserves
  those types instead of reinterpreting the wire projection.
- Asset-id fallback follows Python's 64-character spelling check and
  `bytes.fromhex`, including all six ASCII whitespace characters between
  complete bytes.
  Rust's ASCII whitespace helpers omit vertical tab, so the adapter uses the
  exact `bytes.fromhex` set and rejects non-ASCII whitespace and split bytes.
  It does not
  impose a new 32-decoded-byte requirement at this permissive parser boundary.

The frozen graph corpus grows from 297 to **433** witnesses: 108 nonfinite
mapping cases, 24 additional admission/equality cases and four profile merges.
The retained graph mutations move with their helpers. Twelve new admission
entries bring the catalog to **401**. The initial eleven entries and 14 moved
entries passed targeted mutation runs; the vertical-tab regression failed before
the final correction and the two asset-whitespace entries pass a separate
targeted run. Final full-catalog, workspace and Python results are recorded
in the verification section below.

| New mutation | Named Rust test |
| --- | --- |
| `R2c-admission-infinite-index` | `graph_mapping_nonfinite` |
| `R2c-admission-nan-index` | `graph_mapping_nonfinite` |
| `R2c-admission-txid-before-index` | `graph_mapping_admission` |
| `R2c-admission-input-before-output` | `graph_mapping_nonfinite` |
| `R2c-admission-nan-scalar` | `graph_mapping_admission` |
| `R2c-admission-nan-container` | `graph_mapping_admission` |
| `R2c-admission-dict-nan` | `graph_mapping_admission` |
| `R2c-admission-exact-int-float` | `graph_mapping_admission` |
| `R2c-admission-nonfinite-projection` | `graph_mapping_nonfinite` |
| `R2c-admission-asset-whitespace` | `graph_mapping_admission` |
| `R2c-admission-role-repr` | `graph_mapping_nonfinite` |
| `R2c-admission-asset-vertical-tab` | `graph_mapping_admission` |

## Graph arithmetic

The arithmetic audit replaces bounded native-delta and joint Bitcoin-fee
arithmetic with `BigInt`. Row values first pass Python integer admission; raw
facts never pass to `codec::wide()`. Parsed leg values and output indices stay
unbounded, including cancellations between very large inputs and change.

Findings and fixes:

- Native-zero detection previously discarded quantities outside i64, while
  shared-fee normalization also narrowed output indices and used bounded sums.
  Exact cancellation now proves the same ordinary wallet delta as Python;
  different large output indices cannot collapse into one graph shape.
- Stored fee comparisons use Python numeric equality. `False` and `0.0` equal
  zero; NaN and infinity remain inconsistent present facts. Large int/float
  comparisons retain exact integer precision.
- After scope recovery, native fee completeness, sign and stored-fee consistency
  are checked before walking wallet-delta ownership, preserving which
  malformed-script error is suppressed.
  Row amount/fee conversion still precedes the index-free native-zero no-op.
- Joint normalization parses every observation before checking for a missing
  graph. It checks missing outpoints before hashing scripts, and preserves
  `TypeError` for truthy list/dict script keys before missing-value guards.
- Event groups retain first-seen error order, using an index into an ordered
  vector rather than scanning all earlier groups for every transaction. Fee
  allocation retains the first eligible bearer by transaction id and FIFO
  allocation when no individual bearer exceeds the fee.

The graph corpus now contains **497 witnesses**, including 30 native arithmetic
and 34 joint-fee cases. Six joint-fee scenarios explicitly assert successful
normalization; the others assert refusal, a hold/no-op, or the reference error
family. Five additional full-evidence fixtures check unbounded cancellation,
large shape disagreement, integral float/boolean fees and nonfinite fees through
`custody`, not only the isolated helpers. Snapshot v1 needs no new fact.

Thirty-seven arithmetic mutations supplement the twelve admission mutations;
**438 catalog entries** retain all 389 rules from the base. The original
`D-native-zero-delta` mutation moves to the factored helper's final zero check.
A nonzero-fee mutation initially survived because the later quantity guard also
refused it. A mixed-invalidity witness now proves the earlier fee check suppresses
an unhashable-script error; no survivor is counted as a kill.

Some internal branches have no independent mutation witness: parsed graph
indices always have integer type; JSON cannot produce `Decimal`; a nonempty
successfully parsed event group always has a merged graph; and native delta
values cannot become unknown after the complete-fee admission. Selecting an equal-to-fee bearer instead of the
FIFO path yields the same allocations. These are unreachable or equivalent
boundaries, not claimed mutation kills. Legacy/fake index interfaces remain
outside the decoded schema-1 index, as documented in the decoder checkpoint.

| New arithmetic mutation | Named Rust tests |
| --- | --- |
| `R2c-arithmetic-quantity-precision` | `graph_arithmetic_native`, `graph_arithmetic_fees`, `owned_fee_unbounded_cancellation` |
| `R2c-arithmetic-index-precision` | `graph_arithmetic_fees`, `owned_fee_unbounded_shape` |
| `R2c-arithmetic-nonfinite-fee` | `graph_arithmetic_native`, `graph_arithmetic_fees`, `owned_fee_nan` |
| `R2c-arithmetic-native-fee-equality` | `graph_arithmetic_native` |
| `R2c-arithmetic-owned-fee-equality` | `graph_arithmetic_fees`, `owned_fee_float`, `owned_fee_false` |
| `R2c-arithmetic-fee-before-ownership` | `graph_arithmetic_native` |
| `R2c-arithmetic-row-before-index` | `graph_arithmetic_native` |
| `R2c-arithmetic-native-script-presence` | `graph_arithmetic_native` |
| `R2c-arithmetic-native-outpoint` | `graph_arithmetic_native` |
| `R2c-arithmetic-native-owner-ambiguity` | `graph_arithmetic_native` |
| `R2c-arithmetic-native-owner-membership` | `graph_arithmetic_native` |
| `R2c-arithmetic-native-negative-fee` | `graph_arithmetic_native` |
| `R2c-arithmetic-native-empty-inputs` | `graph_arithmetic_native` |
| `R2c-arithmetic-native-empty-outputs` | `graph_arithmetic_native` |
| `R2c-arithmetic-script-hashability` | `graph_arithmetic_fees` |
| `R2c-arithmetic-parse-all-observations` | `graph_arithmetic_fees` |
| `R2c-arithmetic-group-order` | `graph_arithmetic_fees` |
| `R2c-arithmetic-bitcoin-only` | `graph_arithmetic_fees` |
| `R2c-arithmetic-attribution` | `graph_arithmetic_fees` |
| `R2c-arithmetic-nonzero-recorded-fee` | `graph_arithmetic_fees` |
| `R2c-arithmetic-included-fee` | `graph_arithmetic_fees` |
| `R2c-arithmetic-shape-admission` | `graph_arithmetic_fees`, `owned_fee_unbounded_shape` |
| `R2c-arithmetic-missing-outpoint-before-script` | `graph_arithmetic_fees` |
| `R2c-arithmetic-input-value` | `graph_arithmetic_fees` |
| `R2c-arithmetic-output-value` | `graph_arithmetic_fees` |
| `R2c-arithmetic-delta-conservation` | `graph_arithmetic_fees` |
| `R2c-arithmetic-fee-capacity` | `graph_arithmetic_fees` |
| `R2c-arithmetic-bearer-order` | `graph_arithmetic_fees` |
| `R2c-arithmetic-fifo-fee` | `graph_arithmetic_fees` |
| `R2c-arithmetic-duplicate-wallet` | `graph_arithmetic_fees` |
| `R2c-arithmetic-graph-conflict` | `graph_arithmetic_fees` |
| `R2c-arithmetic-input-owner` | `graph_arithmetic_fees` |
| `R2c-arithmetic-output-owner` | `graph_arithmetic_fees` |
| `R2c-arithmetic-negative-shared-fee` | `graph_arithmetic_fees` |
| `R2c-arithmetic-recorded-fee-match` | `graph_arithmetic_fees` |
| `R2c-arithmetic-inbound-fee` | `graph_arithmetic_fees` |
| `R2c-arithmetic-no-outbound` | `graph_arithmetic_fees` |

## Verification

Rust 1.99.1 and locked Python 3.11 through
`mise exec rust@1.99.1 uv@latest -- ...`, with
`MATURIN_NO_INSTALL_RUST=1`. Cargo runs offline, including the Python harness's
child builds via `CARGO_NET_OFFLINE=true`. The locked environment, books,
recordings, build targets and mutation trees use disposable roots. The binding
was rebuilt after the final asset-whitespace correction.

- `cargo fmt --all --check`, workspace/all-target Clippy with `--locked` and
  warnings denied, and `cargo test --workspace --exclude kassiber-net --locked`
  pass after the final code change: **710 Rust tests**.
- **Every one of the 438 catalog entries is killed** by its named tests, with
  passing unmutated selections and no survivor or nonbuilding mutant counted as
  a kill. Four disjoint scratch-tree runs kill the 437-entry audit catalog
  (110/109/109/109); after the final whitespace correction, both asset-whitespace
  mutations pass again, including the new 438th entry. Catalog validation
  confirms all 438 entries apply. The earlier serial run was interrupted in
  favor of those four runs and is not claimed as complete.

| Mutation group | Entries | Result |
| --- | ---: | --- |
| Retained base rules | 389 | All killed |
| Stored-mapping admission additions | 12 | All killed |
| Graph-arithmetic additions | 37 | All killed |
| Total | **438** | **All killed** |

The retained generator suites compare **12,000 seeded books** with all their
per-scenario activity assertions passing. Their configured main permutations
also pass (separate frozen-fixture permutations are additional):

| Generator | Books | Main permutations | Activity |
| --- | ---: | ---: | --- |
| Evidence | 2,000 | 58 | 345 reconciled books / 690 receipts |
| Components | 2,000 | 2,000 | Per-scenario active/blocked/inactive assertions |
| Samourai | 2,000 | 68 | 1,176 active / 332 blocked / 492 inactive |
| Recorded pairs | 2,000 | 92 | 789 active / 176 blocked / 1,035 inactive |
| Recorded fan-out | 2,000 | 94 | 1,244 active / 126 blocked / 630 inactive |
| Graph prerequisites | 2,000 | 16 | Exactly 250 successful Python results in each of eight scenarios |

The graph scenarios are Bitcoin ownership/consolidation, Liquid
ownership/consolidation, and Liquid ownership with raw bytes, short locktime,
flag 2, or nonminimal input count. Each full Python interpretation succeeds
without blockers/quarantines, producing two consolidation claims or one
rowless native audit. These are Python producer controls; Rust producer and
composed ownership/consolidation coverage remains outstanding.

- The custody Python modules plus ownership-transfer, historical ownership and
  onchain modules under census report **682 passed, 2,679 subtests passed,
  four failed**. Components, evidence and each of the five isolated interpreter
  projections have **214 successful comparisons**, with no differences.
  Interpretation remains skipped in all 215 observed builds. The generated
  suites above are included in these test totals.
- The four failures are
  `CustodyComponentDaemonSurfaceTests.test_daemon_round_trips_unsafe_component_integers_as_strings`,
  `CustodyComponentDaemonSurfaceTests.test_supported_kinds_and_daemon_round_trip`,
  `CustodyGapDaemonProtocolAcceptanceTests.test_real_jsonl_protocol_accepts_opaque_candidate_cursor`
  and `CustodyGapDaemonProtocolAcceptanceTests.test_real_jsonl_protocol_previews_confirms_and_refreshes_gap`.
  They fail on missing daemon response data, as at the previous checkpoint. A
  direct probe confirms `_owner_lock_root` → `os.chmod` →
  `OSError: [Errno 30] Read-only file system` for `~/.kassiber/run`. The owner
  guard remains intact; those tests need a writable owner-runtime environment.
- After the final whitespace change, frozen-vector/snapshot/regression checks
  pass **33 tests**. A further **16 raw-book partial comparisons** pass across
  Bitcoin and Liquid with all ASCII whitespace, non-ASCII whitespace and
  invalid separator controls.
- Recorded integrity checks pass for **206 snapshots**. Replay compares
  **206 requests and 206 permutations**, with no pre-request refusals or
  differences, using a separate target built after the final code change.
- Inventory/contracts checks and their focused **10-test** suite pass. The
  shared JSON-feature comparison reports **145 ledger outputs byte-identical**.
  No dependency, snapshot-schema or raw-fact inventory change is needed.

The initial generated run had one harness failure because a concurrent Cargo
build removed a cached example executable. The final corpus and replay use
dedicated build targets and pass their generated/recorded comparisons; the
initial run is not reported as passing. Full census and custody-shadow shards
remain assigned to the orchestrator. The full quality gate was not run.

## Remaining work

This run stops after the two prerequisite audits. Ownership production,
consolidation production and their partial composition are still unported.
The next slice should port component-asset selection, per-asset conservation,
source ownership predicates, exact Liquid fees, destination reuse/decline and
row cloning into an ownership producer. Consolidation must retain collaborative
own-delta qualification, normalized versus legacy fee handling, all-or-none
recorded destinations and FIFO source/sink cells.

Then compose **consolidation → recorded fan-out → ownership** using
`derive_profile_transfers`' row replacement/drop/synthetic-row rules and its
handled-id updates. Verify the complete derivation and generic claims/native
audits through the partial seam, with per-scenario successful Bitcoin and Liquid
composed controls. The existing graph generator's 250 successful Python producer
results per scenario are prerequisite controls, not Rust producer coverage.

After those producers remain holds/privacy, channels and loans, HTLC/native
transitions, native component reconciliation and final interpreter composition.
Keep `interpretation` skipped throughout those partial slices. Then port gaps,
quantity arbitration and finalized projection, and fuse custody with `ledger`.
