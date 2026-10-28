# R2c Liquid decoder and graph prerequisite checkpoint

Branch `t3code/r2c-interpretation-5` starts at `b61162fc`. Python remains
authoritative. Snapshot schema 1 is unchanged: transaction raw JSON and the
existing scoped ownership-index facts supply this slice. No decoded graph,
ownership verdict, claim, or consolidation result becomes a request fact.
Product responses still list `interpretation` under `skipped_stages`.

## Decoder

`crates/custody/src/liquid.rs` reproduces
`onchain._decode_liquid_raw_hex`, using embit 0.8.0 as the reference. This
operation projects public structure; it does not validate consensus, unblind,
verify proofs, sign, broadcast, or perform I/O. A bounded cursor consumes the
existing byte slice. Length prefixes do not reserve memory or allocate their
declared payload size; every count iteration must consume another prefix or
fail. Reads above CPython's signed 64-bit `Py_ssize_t` limit refuse.

The standard library and existing JSON dependency suffice. No `elements`
dependency, lockfile change, license row, or runtime requirement is added.
The cached 0.25.3 crate's consensus decoder has different admission rules:

| Boundary | Python compatibility behavior | Difference from `elements` 0.25.3 |
| --- | --- | --- |
| Locktime | Accept zero through four remaining bytes | The crate requires all four |
| Witness flag | Only 1 enables witness parsing; every other byte means no witness | The crate rejects flags above 1 |
| Compact integers | Accept nonminimal encodings and short payloads interpreted little endian | The crate requires minimal, complete encodings |
| Proof and stack-item data | Accept a short payload at EOF; scripts still require complete payloads | The crate requires complete byte vectors |
| Empty witnesses | Accept a witness flag with empty witness structures | The crate rejects a superfluous witness flag |
| Output asset | Read 33 bytes without prefix or curve validation; only prefix 1 exposes an asset id | The crate recognizes null, explicit and validated confidential asset encodings |
| Output value | Prefix 1 reads an explicit eight-byte big-endian value; every other prefix reads 32 commitment bytes | The crate distinguishes null and validates confidential prefixes/points |
| Output nonce | Any nonzero prefix consumes 32 bytes and marks the output blinded | The crate distinguishes explicit/confidential nonce variants and validates public keys |
| Issuance commitments | Null and explicit lengths remain strict; every other prefix reads 32 strict bytes without curve validation | The crate validates confidential prefixes and points |
| Coinbase / peg-in / issuance | Keep the coinbase vout sentinel; otherwise mask both flag bits and consume issuance before outputs | The public outpoint projection is pinned independently of crate object methods |
| Trailing data | Refuse | Agrees with the crate |

The 3,744 reference-generated vectors include nonuniform txids and asset ids,
explicit amounts through `u64::MAX`, empty-script fees, confidential outputs
with valid curve commitments, deliberately invalid points, explicit-but-blinded
and confidential-but-unblinded outputs, peg-ins, coinbase, null/explicit/
confidential issuance, both witness stacks and both output proofs, and multiple
inputs and outputs. Every byte truncation of eleven representative structures,
all 256 flags, all 256 prefixes in each output field and issuance commitments,
all compact-integer widths at each count/length position, oversized reads,
trailing bytes, malformed hex and 512 deterministic byte mutations are frozen.
This enumerates parser equivalence classes, not all possible malformed byte
strings. The Python helper catches decoder exceptions and returns `None`;
Rust retains that refusal boundary.

| Vector group | Accepted | Refused |
| --- | ---: | ---: |
| Valid structures | 15 | 0 |
| Every-byte truncations and trailing data | 43 | 1,566 |
| Witness flags | 255 | 1 |
| Output prefixes and invalid points | 771 | 0 |
| Issuance commitment prefixes | 256 | 0 |
| Compact integers | 59 | 241 |
| Witness edge cases | 4 | 7 |
| Hex/type/whitespace boundaries | 2 | 12 |
| Seeded malformed bytes | 310 | 202 |

Version, input txid/vout/sequence, output asset/value/nonce reads cannot be
short in a successfully admitted transaction: a later mandatory compact-prefix
read would fail after they consume EOF. Their short-read behavior is therefore
equivalent at this projection boundary. Strict issuance commitment payload
lengths have the same equivalence: a short amount consumes EOF before the token
commitment prefix, and a short token consumes EOF before the output-count
prefix. Those checks are retained, with no artificial mutation kill claimed.
Peg-in witness bytes do not establish ownership. ScriptSig, proofs, version and locktime are consumed for admission
but are deliberately absent from the Python public graph projection.

## Graph prerequisites

The isolated `partial.interpretation.graph` comparison recomputes parsed graphs,
recovered scopes, input index lookups and profile-scoped sibling merges from
the ordinary raw request. It is exercised by generated books, census/shadow
plumbing and recorded replay; it does not publish an accounting stage.

Graph conflicts now retain Python's exact sorted, deduplicated field labels,
including positional input rekeying, output indices and `owned`/`external`
role refinement. Zero and false remain facts. Mapping values and indices keep
Python integer precision, including the 4,300-digit string admission limit,
and comparisons preserve Python numeric equality.
The existing Liquid raw bytes are decoded after direct, nested and ownership
mapping observations, in Python order.

`graph/ownership.rs` recovers source scope from unscoped outpoint ownership
before script ownership, then uses uniquely scoped owned outputs if the inputs
do not yield one scope. Explicit graph/config chains and networks must agree
with each other and with inference. Blank Liquid networks do not default during
recovery. Liquid component asset identities remain distinct from display codes.
Scoped outpoint lookup honors explicitly empty entries, strictly filters its
legacy fallback and preserves Python's wallet-kind/source/label/id ordering.
Scoped txid lookup never falls back to an unscoped claim. Last duplicate raw
scoped keys win, as in Python snapshot index decoding. Truthy nonstring script
keys preserve `AttributeError`; mixed-invalidity vectors pin txid admission
before scope lookup.

The 297 direct graph witnesses cover mapping, exact conflicts, scope/index
precedence and per-row profile merges. Legacy/fake index interfaces accepting
a single `OwnedMatch` or lacking scoped methods are unreachable through schema
1's decoded index, whose entries are always lists and whose methods are fixed.
The profile map's no-key fallback is unreachable because a parsed row is stored
only after its group key is assigned. No artificial mutation kill is counted
for those Python compatibility branches.

This is a prerequisite checkpoint, not a claim that every permissive Python
stored-mapping edge is ported. Audit raw mapping nonfinite scalars and their
exception precedence before enabling graph ownership. The existing `parse`
adapter's JSON conversion can erase nonfinite values; the new decoder itself
takes hexadecimal text and has no such ambiguity. Also retain the broader
graph-arithmetic admission audit when ownership begins. Receipt coverage,
full/canonical sums and assignments now retain unbounded parsed graph integers
so accepting these facts cannot panic in the existing evidence caller. The
`receipt_unbounded_graph` witness matches Python's targeted hold for inconsistent
coverage. Native-zero-delta and fee arithmetic still need the broader audit.

## Generated successful controls

The added 2,000-book generator asserts exactly 250 active Python producer
results in **each** scenario below. Rust compares decoder/graph prerequisites;
ownership and consolidation production remain unported. Ownership controls
derive a rowless owned destination from an observed funding input. Consolidation
controls contribute two owned sources to one recorded receipt with the shared
network fee conserved. Liquid controls carry explicit component asset ids and
separately identified fee outputs. Full Python interpretation additionally
asserts two target-bearing consolidation claims or one rowless ownership native
audit per book, with no blocked transaction ids or quarantines.

| Scenario | Books with active Python producer | Expected pairs per book |
| --- | ---: | ---: |
| Bitcoin ownership | 250 | 1 |
| Liquid ownership | 250 | 1 |
| Bitcoin consolidation | 250 | 2 |
| Liquid consolidation | 250 | 2 |
| Liquid ownership plus raw bytes | 250 | 1 |
| Liquid ownership plus missing raw locktime | 250 | 1 |
| Liquid ownership plus flag 2 | 250 | 1 |
| Liquid ownership plus nonminimal input count | 250 | 1 |

Sixteen additional permutations reverse unordered index/wallet collections.
The previous 10,000-book generators and their activity assertions are retained.

## Verification

Rust 1.99.1, locked Python 3.11, `MATURIN_NO_INSTALL_RUST=1`; Cargo runs offline.
All books, recordings and mutation trees use disposable roots. Full census and
custody-shadow shards remain assigned to the orchestrator.

- `cargo fmt --all --check`, workspace Clippy with all targets, locked and
  warnings denied, and offline workspace tests excluding `kassiber-net` pass:
  **701 Rust tests**. The binding was rebuilt from the locked environment after
  the final graph integer-admission guard.
- The generated suites pass **14 tests**, comparing **12,000 seeded books**
  and their configured permutations. The new graph generator also passes a
  separate run with its strengthened full-Python-interpreter assertions:
  exactly 250 successful results in all eight scenarios, with two consolidation
  claims or one rowless native audit and no blockers/quarantines in each book.
- Recorded replay compares **195 requests and 195 permutations**, with zero
  differences or pre-request refusals. Their snapshot-integrity check also
  passes. Replay was repeated after the final integer guard.
- The focused custody, ownership, onchain and historical-compatibility modules
  under census report **693 passed, 2,667 subtests passed, four failed**.
  Components, evidence and each isolated interpreter projection have 203
  successful comparisons, with no difference. These counts exclude the three
  generated modules above. A further **121 tests and 12 subtests** covering
  frozen vectors, snapshot contracts, ownership transfers and onchain parsing
  pass after the final integer guard.
- The four failures are the two `CustodyComponentDaemonSurfaceTests` and two
  `CustodyGapDaemonProtocolAcceptanceTests` that also failed before this
  checkpoint. A separate probe reproduces `_owner_lock_root` -> `os.chmod` ->
  `OSError: [Errno 30] Read-only file system` for `~/.kassiber/run`. The owner
  guard was not relaxed or redirected. These failures remain for orchestration
  in a writable owner-runtime environment.
- Inventory and contracts update/check and shard validation pass; the focused
  inventory/contracts suite passes **10 tests**. The earlier contract, ledger,
  report and agent-interface suite passes **368 tests and seven subtests**.
  The shared JSON-feature check reports **145 ledger outputs byte-identical**.
- **Every one of the 389 catalog entries is killed.** The full 388-entry
  sweep passes; after adding the integer-digit guard, all **71 additions**
  pass again on the final tree (including the adapted integer-conversion
  mutation). Catalog validation reports 389 applicable entries. The removed
  issuance-length mutation is projection-equivalent for the reason above,
  rather than counted as a kill.

This is a local checkpoint, not a full quality-gate or complete R2c result.
The remaining graph admission audit and interpreter producers below must finish
before interpreting this branch as a complete stage.

## Remaining slices

1. Finish the stored-mapping/graph admission audit noted above, and carry the
   graph predicates for source contradictions, asset relation and native wallet
   deltas into the ownership producer with named mutation witnesses.
2. Port ownership interpretation, then multi-source consolidation and the exact
   consolidation -> fan-out -> ownership row/handled-id composition.
3. Port raw holds/privacy, channel lifecycle/loans, HTLC transition production,
   native component reconciliation and final interpretation composition.
4. Keep interpretation skipped until the complete stage passes; then continue
   gaps, quantity arbitration, finalized projection and the journal operation.

## Mutation table

The retained catalog has 318 entries. This checkpoint adds 28 decoder, 42 graph
and one receipt precision entry, for **389** total. Named killing selections
for the additions follow; the JSON catalog remains executable source of truth.

| Mutation | Named killing tests |
| --- | --- |
| `R2c-liquid-short-locktime` | `liquid_truncation` |
| `R2c-liquid-flag` | `liquid_flags` |
| `R2c-liquid-nonminimal` | `liquid_varints` |
| `R2c-liquid-short-varint` | `liquid_varints` |
| `R2c-liquid-short-proof` | `liquid_witness` |
| `R2c-liquid-script-length` | `liquid_truncation` |
| `R2c-liquid-trailing` | `liquid_truncation` |
| `R2c-liquid-asset-prefix` | `liquid_prefixes` |
| `R2c-liquid-value-prefix` | `liquid_prefixes` |
| `R2c-liquid-nonce-prefix` | `liquid_prefixes` |
| `R2c-liquid-empty-witness` | `liquid_witness` |
| `R2c-liquid-issuance-prefix` | `liquid_issuance` |
| `R2c-liquid-issuance-nonce` | `liquid_valid` |
| `R2c-liquid-vout-flags` | `liquid_valid` |
| `R2c-liquid-coinbase` | `liquid_valid` |
| `R2c-liquid-txid-order` | `liquid_valid` |
| `R2c-liquid-value-order` | `liquid_valid` |
| `R2c-liquid-asset-order` | `liquid_valid` |
| `R2c-liquid-nonce-blinding` | `liquid_valid` |
| `R2c-liquid-asset-id` | `liquid_prefixes` |
| `R2c-liquid-fee-role` | `liquid_valid` |
| `R2c-liquid-input-witness` | `liquid_witness` |
| `R2c-liquid-output-witness` | `liquid_witness` |
| `R2c-liquid-hex-whitespace` | `liquid_hex` |
| `R2c-liquid-read-overflow` | `liquid_varints` |
| `R2c-liquid-null-issuance` | `liquid_valid` |
| `R2c-liquid-explicit-issuance` | `liquid_valid` |
| `R2c-graph-conflict-label` | `graph_merge_conflicts` |
| `R2c-graph-header-conflict` | `graph_merge_conflicts` |
| `R2c-graph-role-refinement` | `graph_merge_conflicts` |
| `R2c-graph-zero-present` | `graph_merge_conflicts` |
| `R2c-graph-python-equality` | `graph_merge_conflicts` |
| `R2c-graph-first-conflicts` | `graph_merge_conflicts` |
| `R2c-graph-position-rekey` | `graph_merge_conflicts` |
| `R2c-graph-output-order` | `graph_merge_conflicts` |
| `R2c-graph-sats-int` | `graph_mapping_values` |
| `R2c-graph-raw-hex` | `graph_mapping_values` |
| `R2c-graph-raw-hex-gate` | `graph_mapping_values` |
| `R2c-index-empty-scoped` | `graph_index_precedence` |
| `R2c-index-legacy-scope` | `graph_index_precedence` |
| `R2c-index-outpoint-order-kind` | `graph_index_precedence` |
| `R2c-index-outpoint-order-source` | `graph_index_precedence` |
| `R2c-index-outpoint-order-label` | `graph_index_precedence` |
| `R2c-index-outpoint-order-id` | `graph_index_precedence` |
| `R2c-index-outpoint-precedence` | `graph_index_precedence` |
| `R2c-index-source-precedence` | `graph_scope_recovery` |
| `R2c-index-script-type` | `graph_index_precedence` |
| `R2c-index-outpoint-last-key` | `graph_index_precedence` |
| `R2c-index-txid-last-key` | `graph_index_precedence` |
| `R2c-index-txid-no-legacy` | `graph_index_precedence` |
| `R2c-index-txid-scope-alias` | `graph_index_precedence` |
| `R2c-scope-source-wallet` | `graph_scope_recovery` |
| `R2c-scope-source-ambiguity` | `graph_scope_recovery` |
| `R2c-scope-output-fallback` | `graph_scope_recovery` |
| `R2c-scope-output-ambiguity` | `graph_scope_recovery` |
| `R2c-scope-config-fallback` | `graph_scope_recovery` |
| `R2c-scope-chain-contradiction` | `graph_scope_recovery` |
| `R2c-scope-network-contradiction` | `graph_scope_recovery` |
| `R2c-scope-inferred-contradiction` | `graph_scope_recovery` |
| `R2c-scope-liquid-no-default` | `graph_scope_recovery` |
| `R2c-scope-txid-invalid` | `graph_scope_recovery` |
| `R2c-scope-txid-conflict` | `graph_scope_recovery` |
| `R2c-scope-asset-domain` | `graph_scope_recovery` |
| `R2c-scope-liquid-asset-id` | `graph_scope_recovery` |
| `R2c-profile-group-network` | `graph_profile_merge` |
| `R2c-profile-row-first` | `graph_profile_merge` |
| `R2c-profile-component-first` | `graph_profile_merge` |
| `R2c-profile-duplicate-id` | `graph_profile_merge` |
| `R2c-liquid-curve-point` | `liquid_prefixes` |
| `R2c-receipt-graph-precision` | `receipt_unbounded_graph` |
| `R2c-graph-integer-digit-limit` | `graph_mapping_values` |
