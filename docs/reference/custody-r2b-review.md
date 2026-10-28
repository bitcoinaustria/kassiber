# R2b components review fixes

Branch: `t3code/r2b-fixes`, based on `6da3e459`. Local commits only.
Python remains authoritative. No dependencies, snapshot fields, accounting
policies, production Python behavior, network access, or storage formats change.

## Findings

| Finding | Resolution and regression |
| --- | --- |
| P2-1 | All seven evidence/graph `.upper()` mirrors use `py_upper`. Resealed U+019B and U+0264 row assets stay matched and active; `components_asset_hash_upper` failed before the fix. |
| P2-2 | Raw revisions and anchor quantities use fallible integer conversion. `wide()` takes converted integers only, with a debug assertion. Four raw-fact refusal vectors failed before the fix; the audit below covers every caller. |
| P2-3 | Mirror the trailing-Z rewrite, CPython offset whitespace/fraction handling and ambiguous week-date separator rules. The expanded Cartesian corpus initially found 1,105 mismatches. All 45,477 cases now pass, including 9,477 seeded differential fuzz cases regenerated from CPython 3.11. |
| P2-4 | Fix scaled suspense and its observed fee, reseal balanced conversions, and include the second outbound anchor for N:M allocation. Assert active counts inside the generator. Every requested positive shape activates. |
| P3-1 | Globally sort raw quantity legs before conversion; convert every leg before any allocation. Both refusal-order regressions failed before the fix. Raw integer sort keys retain precision above 2^53. |
| P3-2 | Convert expected evidence count before ordinals; leave ordinals unread when the count is null. Both precedence regressions failed before the fix. |
| P3-3 | Use typed evidence errors and explicit Python catch families. Native support skips only TypeError/ValueError (including UnicodeEncodeError, a ValueError subclass); evidence availability additionally catches KeyError/SQLite errors. Compute commitment detail hashes outside the canonical-evidence catch. Frozen component vectors compare complete `detail` strings; the catch-family test pins propagation and unchanged detail text. |
| P3-4 | Validate exact and legacy rate decimals before stages, preserving first-successful-asset precedence and parsing both rate/value columns in Python order. Three malformed-rate regressions failed before the fix; a later duplicate junk row stays ignored. Recorded replay applies the same Python admission check. |
| P3-5 | Documented why dedup is unreachable in this Rust helper: earlier issues are anchor/coverage issues; the only allocation-network issues are appended at the single scope call. The Python helper accepts a broader issue list. |
| P3-6 (optional) | Added typed errors; deferred a full replacement of stage-boundary JSON maps to keep this correction scoped to parity. |

## Complete wide audit

| Callers | Findings |
| --- | --- |
| `components/mod.rs` | Remove `wide` from raw header sort keys and competing-header projections. Convert revision link targets at Python's `int()` point. Child counts are converted before arithmetic. |
| `components/anchors.rs` | Convert raw amount/fee at boundary normalization and suspense fee coverage, including excluded anchors absent from the native population. Match Python's explicit `or 0`; malformed truthy facts refuse. Other calls consume projected legs/allocations. |
| `components/commitments.rs` | Expected count/ordinal are converted in reference order; native legs/allocations are projected integers. Native principal arithmetic now propagates conversion failures. |
| `components/conservation.rs` | Calls consume projected integers or computed balance rows. Two nullable valuation tests previously relied on the implicit zero fallback; they now explicitly default null to zero after projection. Balance aggregation already skips null valuations. |
| `components/scope.rs` | Calls consume projected legs/allocations or inferred edges built from those projected values. No raw numeric reads remain. |
| `evidence.rs` | Duplicate-fee adjustment had a raw amount read; convert with fallible `integer_or_zero`. Observation validation and deterministic observation selection consume quantities converted during observation construction. |
| `graph.rs` | Receipt validation had a raw row amount read; convert fallibly. Output `value_sats` is already the typed result of graph `sats()` parsing and checked for missing values before arithmetic. |
| `codec.rs` | `wide` no longer parses arbitrary facts or defaults failures to zero. It asserts integer JSON input and requires representable converted arithmetic values. |

## Verification

Tooling: Rust 1.99.1, locked Python 3.11, `MATURIN_NO_INSTALL_RUST=1`.
Cargo used the local offline cache. Test books and recordings used `/tmp`.

| Check | Result |
| --- | --- |
| `cargo fmt --all --check` | Passed. |
| `cargo clippy --workspace --all-targets --locked -- -D warnings` | Passed (offline). |
| `cargo test --workspace --exclude kassiber-net --locked` | 440 passed (offline). |
| Custody mutation runner | All 167 killed after 147 named baseline tests passed unmutated. No survivors, nonbuilding mutations or empty selections. |
| Generated comparisons | 4,000 requests passed: 2,000 original evidence books and 2,000 component books. |
| Component permutations | 2,000 Python/Rust permutations passed; generated/frozen tests: 7 passed. |
| Recorded input checks | 195 snapshots, zero integrity problems. |
| Recorded comparisons | 195 requests and 195 permutations, zero differences. |
| Custody-focused Python modules | 497 passed and 2,655 subtests passed; four daemon tests hit the sandbox owner-lock restriction below. |
| Inventory | `scripts/python_test_shards.py validate` passed. |
| Inventory/contract tests | 368 passed and 7 subtests passed across Rust-port inventory/contracts, tax-ledger, report drift and CLI-agent contracts. |
| Ledger feature contract | All 145 ledger responses byte-identical with and without custody JSON features. |
| Full census / custody-shadow shards | Delegated to the orchestrator outside this sandbox as requested; not claimed as local verification. |

The sandbox refuses the unchanged stable per-user owner-lock directory
`~/.kassiber/run` (read-only filesystem). These four real daemon subprocess
tests therefore could not complete; the diagnostic stack points to
`operator/project.py::_owner_lock_root`, before the requested daemon operation:

- `CustodyComponentDaemonSurfaceTests::test_daemon_round_trips_unsafe_component_integers_as_strings`
- `CustodyComponentDaemonSurfaceTests::test_supported_kinds_and_daemon_round_trip`
- `CustodyGapDaemonProtocolAcceptanceTests::test_real_jsonl_protocol_accepts_opaque_candidate_cursor`
- `CustodyGapDaemonProtocolAcceptanceTests::test_real_jsonl_protocol_previews_confirms_and_refreshes_gap`

`kassiber-net` tests were excluded as requested. No push, PR, or full quality-gate
claim is made. Manual second-pass review covered source diffs and raw/conversion
boundaries. The first mutation run exposed two surviving alias tests; conflicting
scope scenarios now require those inferences and kill their mutations.

## Mutation table

X07, X14 and X15 retain the reviewer's equivalent/loader-equivalent disposition:
they do not expose an independently observable change on admitted stage facts,
so no artificial nonbuilding or unreachable mutation is counted as a kill.
Every other requested X-id has a named test below. Existing entries are retained.

| Mutation | Named killing tests | Result |
| --- | --- | --- |
| `A-duplicate-fee-keeper` | `duplicate_fee_keeper` | Killed |
| `B-owner-deprecated` | `receipt_deprecated` | Killed |
| `C-authority-hashes` | `tampered_graph`, `tampered_quantity` | Killed |
| `D-native-zero-delta` | `zero_delta` | Killed |
| `E-aggregate-leg-tie` | `duplicate_slot` | Killed |
| `F-overcount-requires-address` | `nonaddress_overcount` | Killed |
| `owner-priority` | `receipt_priority` | Killed |
| `owner-label` | `receipt_label` | Killed |
| `owner-id` | `receipt_id` | Killed |
| `onchain-key-alnum` | `cpython_onchain_scopes` | Killed |
| `components-no-legs` | `components_no_legs` | Killed |
| `components-source-total` | `components_missing_source` | Killed |
| `components-destination-total` | `components_missing_destination` | Killed |
| `components-unknown-role` | `components_unknown_role` | Killed |
| `components-fee-asset` | `components_fee_asset` | Killed |
| `components-fee-wallet` | `components_fee_wallet` | Killed |
| `components-fee-scope` | `components_fee_scope` | Killed |
| `components-conversion-fee-quantity` | `components_conversion_fee_quantity` | Killed |
| `components-conversion-fee-value` | `components_conversion_fee_value` | Killed |
| `components-allocation-endpoints` | `components_allocation_invalid` | Killed |
| `components-allocation-quantity` | `components_allocation_quantity` | Killed |
| `components-source-coverage` | `components_allocation_source_coverage` | Killed |
| `components-sink-coverage` | `components_allocation_sink_coverage` | Killed |
| `components-orphan-fee` | `components_fee_orphan` | Killed |
| `components-allocation-required` | `components_allocation_required` | Killed |
| `components-suspense-mode` | `components_suspense_mode` | Killed |
| `components-suspense-review` | `components_suspense_review` | Killed |
| `components-suspense-allocations` | `components_suspense_no_allocations` | Killed |
| `components-suspense-anchor` | `components_suspense_anchor` | Killed |
| `components-suspense-time-required` | `components_suspense_missing_time` | Killed |
| `components-suspense-observed-source` | `components_suspense_source` | Killed |
| `components-suspense-asset` | `components_suspense_asset` | Killed |
| `components-suspense-quantity` | `components_suspense_quantity` | Killed |
| `components-suspense-network` | `components_suspense_network` | Killed |
| `components-suspense-time` | `components_suspense_time` | Killed |
| `components-value-only-loss` | `components_value_only_loss` | Killed |
| `components-missing-time` | `components_missing_time` | Killed |
| `components-invalid-time` | `components_invalid_time` | Killed |
| `components-quantity-balance` | `components_unbalanced` | Killed |
| `components-conversion-topology` | `components_conversion_topology` | Killed |
| `components-retained-quantity` | `components_conversion_retained` | Killed |
| `components-conversion-review` | `components_conversion_review` | Killed |
| `components-conversion-policy` | `components_conversion_policy` | Killed |
| `components-conversion-values` | `components_conversion_missing_value` | Killed |
| `components-conversion-no-values` | `components_conversion_no_values` | Killed |
| `components-conversion-balance` | `components_conversion_unbalanced` | Killed |
| `components-owned-wallet` | `components_owned_wallet_missing` | Killed |
| `components-retracted-anchor` | `components_anchor_retracted` | Killed |
| `components-anchor-identity` | `components_anchor_identity` | Killed |
| `components-missing-anchor` | `components_anchor_missing` | Killed |
| `components-excluded-anchor` | `components_anchor_excluded` | Killed |
| `components-source-direction` | `components_anchor_source_direction` | Killed |
| `components-destination-direction` | `components_anchor_destination_direction` | Killed |
| `components-loss-direction` | `components_anchor_loss_direction` | Killed |
| `components-anchor-asset` | `components_anchor_asset` | Killed |
| `components-anchor-wallet` | `components_anchor_wallet` | Killed |
| `components-anchor-time-invalid` | `components_anchor_time_invalid` | Killed |
| `components-anchor-time` | `components_anchor_time_mismatch` | Killed |
| `components-observer-time-warning` | `components_anchor_time_warning` | Killed |
| `components-partial-lower-bound` | `components_anchor_partial_zero` | Killed |
| `components-partial-upper-bound` | `components_anchor_partial_excess` | Killed |
| `components-full-coverage` | `components_anchor_coverage` | Killed |
| `components-suspense-observed-fee` | `components_suspense_fee` | Killed |
| `components-transaction-required` | `components_transaction_anchor_missing` | Killed |
| `components-reviewed-location` | `components_retained_location` | Killed |
| `components-scope-enrichment` | `components_scope_enriched_mismatch` | Killed |
| `components-lineage-active` | `components_lineage_conflict` | Killed |
| `components-revision-profile` | `components_revision_scope` | Killed |
| `components-revision-lineage` | `components_revision_lineage` | Killed |
| `components-revision-order` | `components_revision_order` | Killed |
| `components-revision-forward` | `components_revision_forward` | Killed |
| `components-membership-coalesce` | `components_membership_empty_anchor` | Killed |
| `components-membership-conflict` | `components_membership_conflict` | Killed |
| `components-child-commitments` | `components_content_missing` | Killed |
| `components-child-counts` | `components_leg_count` | Killed |
| `components-evidence-blocker` | `components_commitment_hash` | Killed |
| `components-native-blocker` | `components_native_contradicted` | Killed |
| `components-effective-state` | `components_no_legs` | Killed |
| `components-blocker-membership` | `components_balanced` | Killed |
| `components-blocker-precedence` | `components_no_legs` | Killed |
| `components-blocker-code-order` | `components_no_legs` | Killed |
| `components-header-id-tie` | `components_component_order` | Killed |
| `components-leg-order` | `components_children_order` | Killed |
| `components-allocation-order` | `components_children_order` | Killed |
| `components-location-redaction` | `components_retained_location` | Killed |
| `components-commitment-header` | `components_commitment_header` | Killed |
| `components-commitment-count` | `components_commitment_count` | Killed |
| `components-commitment-ordinal` | `components_commitment_ordinals` | Killed |
| `components-zero-commitment-anchors` | `components_commitment_zero` | Killed |
| `components-commitment-missing-anchors` | `components_commitment_no_anchors` | Killed |
| `components-commitment-match` | `components_commitment_hash` | Killed |
| `components-native-capacity` | `components_native_contradicted` | Killed |
| `components-native-equality` | `components_native_supported` | Killed |
| `components-native-asset` | `components_native_other_asset` | Killed |
| `components-native-exact-sink` | `components_native_exact_sink` | Killed |
| `components-volatile-fingerprint` | `components_commitment_volatile` | Killed |
| `components-volatile-occurred_at` | `components_commitment_volatile` | Killed |
| `components-volatile-confirmed_at` | `components_commitment_volatile` | Killed |
| `components-volatile-raw_json` | `components_commitment_volatile` | Killed |
| `components-hash-detail-schema` | `components_balanced` | Killed |
| `components-hash-detail-quantity` | `components_balanced` | Killed |
| `components-hash-detail-kind` | `components_hash_kind` | Killed |
| `components-hash-detail-payment` | `components_hash_payment` | Killed |
| `components-hash-detail-payment-source` | `components_hash_payment_source` | Killed |
| `components-hash-detail-refund-txid` | `components_hash_refund_txid` | Killed |
| `components-hash-detail-refund-vout` | `components_hash_refund_vout` | Killed |
| `components-hash-quantity-schema` | `components_balanced` | Killed |
| `components-hash-quantity-event` | `components_hash_event` | Killed |
| `components-hash-quantity-wallet` | `components_hash_wallet` | Killed |
| `components-hash-quantity-direction` | `components_hash_direction` | Killed |
| `components-hash-quantity-asset` | `components_hash_asset` | Killed |
| `components-hash-amount_msat` | `components_hash_amount` | Killed |
| `components-hash-fee_msat` | `components_hash_fee` | Killed |
| `components-hash-amount_includes_fee` | `components_hash_included` | Killed |
| `components-anchor-rail` | `components_anchor_rail` | Killed |
| `components-anchor-chain` | `components_anchor_chain` | Killed |
| `components-anchor-network` | `components_anchor_network` | Killed |
| `components-invalid-chain` | `components_scope_invalid_chain` | Killed |
| `components-rail-chain` | `components_scope_rail_chain` | Killed |
| `components-network-domain` | `components_scope_mismatch` | Killed |
| `components-invalid-network` | `components_scope_invalid_network` | Killed |
| `components-chronology-skew` | `components_chronology` | Killed |
| `components-location-capacity` | `components_location_continuity` | Killed |
| `components-connectivity-domain` | `components_scope_laundering` | Killed |
| `components-cross-route` | `components_cross_scope_laundering` | Killed |
| `components-cross-continuity` | `components_cross_location_continuity` | Killed |
| `components-location-tie` | `components_location_ties` | Killed |
| `components-revision-missing` | `components_revision_missing` | Killed |
| `components-revision-conflict-tie` | `components_lineage_ties` | Killed |
| `components-revision-conflict-order` | `components_lineage_ties` | Killed |
| `components-transactionless-wallet` | `components_transactionless_wallet_missing` | Killed |
| `components-hash-event-chain` | `components_balanced` | Killed |
| `components-hash-event-network` | `components_balanced` | Killed |
| `components-hash-event-namespace` | `components_balanced` | Killed |
| `components-hash-event-id` | `components_balanced` | Killed |
| `components-hash-utf8` | `components_hash_kind` | Killed |
| `components-header-revision-tie` | `components_lineage_ties` | Killed |
| `components-integer-type-error` | `components_refusal_list_integer` | Killed |
| `components-utc-overflow` | `components_refusal_utc_underflow` | Killed |
| `components-chronology-overflow` | `components_refusal_chronology_overflow` | Killed |
| `components-asset-uppercase` | `components_asset_upper_semantics` | Killed |
| `components-unicode-decimal` | `cpython_component_integers` | Killed |
| `components-refusal-precedence` | `components_refusal_child_before_mode` | Killed |
| `X01-asset-hash-cpython-upper` | `components_asset_hash_upper` | Killed |
| `X02-excluded-anchor-integer` | `components_suspense_fee` | Killed |
| `X03-wallet-scope-fallback` | `components_wallet_scope_conflict` | Killed |
| `X04-nwc-alias` | `components_lightning_receive_nwc` | Killed |
| `X05-core-ln-alias` | `components_core_ln_kind_scope` | Killed |
| `X06-l-btc-alias` | `components_asset_l_btc_alias` | Killed |
| `X09-conversion-ambiguity` | `components_conversion_ambiguous` | Killed |
| `X10-conversion-inferred-pairs` | `components_conversion_inferred_chronology` | Killed |
| `X11-authored-source-default` | `components_authored_source_default` | Killed |
| `X12-native-namespace-gate` | `components_native_namespace_gate` | Killed |
| `X13-native-excluded-rows` | `components_native_excluded_rows` | Killed |
| `X16-competing-profile-filter` | `components_lineage_other_profile` | Killed |
| `X17-zero-amount-chronology` | `components_zero_amount_chronology` | Killed |
| `X18-reviewed-residual-notes` | `components_retained_reviewed_notes` | Killed |
| `X19-lightning-kind-inference` | `components_lightning_kind_scope` | Killed |
| `X20-liquid-substring-kind` | `components_liquid_kind_conflict` | Killed |
| `X22-lightning-chain-default` | `components_lightning_chain_default` | Killed |
| `X23-generic-elements-domain` | `components_generic_elements_domain` | Killed |
| `X24-fee-null-domain-guard` | `components_fee_null_domain` | Killed |
| `X25-authoritative-chronology` | `components_authoritative_chronology` | Killed |
| `X26-empty-time-absent` | `components_missing_time_empty` | Killed |
| `X27-empty-transaction-id` | `components_transaction_id_empty` | Killed |
| `rates-before-components` | `components_refusal_rate_exact` | Killed |
| `commitment-catch-family` | `component_exception_catch_families` | Killed |

## Per-scenario component counts

Over 2,000 seeds, 156 scenarios produce 474 active, 1,629 blocked and 13 inactive
authored components (2,103 materialized components). Inactive means an authored
header filtered out before materialization. Each generator call asserts the
scenario's expected active count, including zero for negative controls.

| Scenario | Books | Active | Blocked | Inactive |
| --- | ---: | ---: | ---: | ---: |
| `balanced` | 13 | 13 | 0 | 0 |
| `no_legs` | 13 | 0 | 13 | 0 |
| `missing_source` | 13 | 0 | 13 | 0 |
| `missing_destination` | 13 | 0 | 13 | 0 |
| `unknown_role` | 13 | 0 | 13 | 0 |
| `unbalanced` | 13 | 0 | 13 | 0 |
| `unit_mismatch` | 13 | 0 | 13 | 0 |
| `missing_time` | 13 | 0 | 13 | 0 |
| `invalid_time` | 13 | 0 | 13 | 0 |
| `chronology` | 13 | 0 | 13 | 0 |
| `location_continuity` | 13 | 0 | 13 | 0 |
| `allocation_required` | 13 | 0 | 13 | 0 |
| `multi_allocation` | 13 | 13 | 0 | 0 |
| `allocation_invalid` | 13 | 0 | 13 | 0 |
| `allocation_quantity` | 13 | 0 | 13 | 0 |
| `allocation_source_coverage` | 13 | 0 | 13 | 0 |
| `allocation_sink_coverage` | 13 | 0 | 13 | 0 |
| `fee_asset` | 13 | 0 | 13 | 0 |
| `fee_wallet` | 13 | 0 | 13 | 0 |
| `fee_scope` | 13 | 0 | 13 | 0 |
| `fee_orphan` | 13 | 0 | 13 | 0 |
| `scope_invalid_chain` | 13 | 0 | 13 | 0 |
| `scope_rail_chain` | 13 | 0 | 13 | 0 |
| `scope_invalid_network` | 13 | 0 | 13 | 0 |
| `scope_mismatch` | 13 | 0 | 13 | 0 |
| `scope_laundering` | 13 | 0 | 13 | 0 |
| `cross_scope_laundering` | 13 | 0 | 26 | 0 |
| `cross_location_continuity` | 13 | 0 | 26 | 0 |
| `conversion` | 13 | 13 | 0 | 0 |
| `conversion_topology` | 13 | 0 | 13 | 0 |
| `conversion_retained` | 13 | 0 | 13 | 0 |
| `conversion_review` | 13 | 0 | 13 | 0 |
| `conversion_policy` | 13 | 0 | 13 | 0 |
| `conversion_missing_value` | 13 | 0 | 13 | 0 |
| `conversion_no_values` | 13 | 0 | 13 | 0 |
| `conversion_unbalanced` | 13 | 0 | 13 | 0 |
| `conversion_fee_quantity` | 13 | 0 | 13 | 0 |
| `conversion_fee_value` | 13 | 0 | 13 | 0 |
| `value_only_loss` | 13 | 0 | 13 | 0 |
| `suspense` | 13 | 13 | 0 | 0 |
| `suspense_mode` | 13 | 0 | 13 | 0 |
| `suspense_review` | 13 | 0 | 13 | 0 |
| `suspense_no_allocations` | 13 | 0 | 13 | 0 |
| `suspense_anchor` | 13 | 0 | 13 | 0 |
| `suspense_missing_time` | 13 | 0 | 13 | 0 |
| `suspense_source` | 13 | 0 | 13 | 0 |
| `suspense_asset` | 13 | 0 | 13 | 0 |
| `suspense_quantity` | 13 | 0 | 13 | 0 |
| `suspense_network` | 13 | 0 | 13 | 0 |
| `suspense_time` | 13 | 0 | 13 | 0 |
| `suspense_fee` | 13 | 0 | 13 | 0 |
| `content_missing` | 13 | 0 | 13 | 0 |
| `leg_count` | 13 | 0 | 13 | 0 |
| `allocation_count` | 13 | 0 | 13 | 0 |
| `term_count` | 13 | 0 | 13 | 0 |
| `commitment_header` | 13 | 0 | 13 | 0 |
| `commitment_count` | 13 | 0 | 13 | 0 |
| `commitment_ordinals` | 13 | 0 | 13 | 0 |
| `commitment_zero` | 13 | 0 | 13 | 0 |
| `commitment_no_anchors` | 13 | 0 | 13 | 0 |
| `commitment_hash` | 13 | 0 | 13 | 0 |
| `commitment_volatile` | 13 | 0 | 13 | 0 |
| `commitment_rejected` | 13 | 0 | 13 | 0 |
| `anchor_missing` | 13 | 0 | 13 | 0 |
| `anchor_retracted` | 13 | 0 | 13 | 0 |
| `anchor_identity` | 13 | 0 | 13 | 0 |
| `anchor_excluded` | 13 | 0 | 13 | 0 |
| `anchor_source_direction` | 13 | 0 | 13 | 0 |
| `anchor_destination_direction` | 13 | 0 | 13 | 0 |
| `anchor_loss_direction` | 13 | 0 | 13 | 0 |
| `anchor_asset` | 13 | 0 | 13 | 0 |
| `anchor_wallet` | 13 | 0 | 13 | 0 |
| `anchor_time_invalid` | 13 | 0 | 13 | 0 |
| `anchor_time_mismatch` | 13 | 0 | 13 | 0 |
| `anchor_time_warning` | 13 | 0 | 13 | 0 |
| `anchor_coverage` | 13 | 0 | 13 | 0 |
| `anchor_partial` | 13 | 0 | 13 | 0 |
| `anchor_partial_zero` | 13 | 0 | 13 | 0 |
| `anchor_partial_excess` | 13 | 0 | 13 | 0 |
| `anchor_rail` | 13 | 0 | 13 | 0 |
| `anchor_chain` | 13 | 0 | 13 | 0 |
| `anchor_network` | 13 | 0 | 13 | 0 |
| `owned_wallet_missing` | 13 | 0 | 13 | 0 |
| `retained_location` | 13 | 0 | 13 | 0 |
| `transaction_anchor_missing` | 13 | 0 | 13 | 0 |
| `lineage_conflict` | 13 | 0 | 26 | 0 |
| `revision_missing` | 13 | 0 | 13 | 0 |
| `revision_scope` | 13 | 0 | 13 | 0 |
| `revision_lineage` | 13 | 0 | 13 | 0 |
| `revision_order` | 13 | 0 | 13 | 0 |
| `revision_forward` | 13 | 0 | 13 | 0 |
| `revision_valid` | 13 | 13 | 0 | 0 |
| `membership_conflict` | 13 | 0 | 26 | 0 |
| `membership_empty_anchor` | 13 | 0 | 26 | 0 |
| `component_order` | 13 | 0 | 26 | 0 |
| `children_order` | 13 | 13 | 0 | 0 |
| `native_supported` | 13 | 13 | 0 | 0 |
| `native_partial` | 13 | 0 | 13 | 0 |
| `native_contradicted` | 13 | 0 | 13 | 0 |
| `native_exact_sink` | 13 | 13 | 0 | 0 |
| `native_other_asset` | 13 | 0 | 13 | 0 |
| `hash_kind` | 13 | 13 | 0 | 0 |
| `hash_payment` | 13 | 13 | 0 | 0 |
| `hash_payment_source` | 13 | 13 | 0 | 0 |
| `hash_refund_txid` | 13 | 13 | 0 | 0 |
| `hash_refund_vout` | 13 | 13 | 0 | 0 |
| `hash_wallet` | 13 | 0 | 13 | 0 |
| `hash_direction` | 13 | 0 | 13 | 0 |
| `hash_asset` | 13 | 0 | 13 | 0 |
| `hash_amount` | 13 | 0 | 13 | 0 |
| `hash_fee` | 13 | 0 | 13 | 0 |
| `hash_included` | 13 | 13 | 0 | 0 |
| `hash_event` | 13 | 13 | 0 | 0 |
| `integer_semantics` | 13 | 13 | 0 | 0 |
| `time_semantics` | 13 | 13 | 0 | 0 |
| `scope_enriched_mismatch` | 13 | 0 | 13 | 0 |
| `lineage_ties` | 13 | 0 | 39 | 0 |
| `location_ties` | 13 | 0 | 13 | 0 |
| `transactionless_wallet_missing` | 13 | 0 | 13 | 0 |
| `anchor_row_time_invalid` | 13 | 0 | 13 | 0 |
| `revision_backward` | 13 | 0 | 13 | 0 |
| `chronology_after_boundary` | 13 | 0 | 13 | 0 |
| `asset_l_btc_alias` | 13 | 13 | 0 | 0 |
| `lightning_chain_default` | 13 | 0 | 13 | 0 |
| `lightning_kind_scope` | 13 | 0 | 13 | 0 |
| `core_ln_kind_scope` | 13 | 0 | 13 | 0 |
| `liquid_kind_conflict` | 13 | 0 | 13 | 0 |
| `inactive_header` | 13 | 0 | 0 | 13 |
| `peg_in` | 12 | 12 | 0 | 0 |
| `peg_out` | 12 | 12 | 0 | 0 |
| `lightning_receive_lnd` | 12 | 12 | 0 | 0 |
| `lightning_receive_nwc` | 12 | 12 | 0 | 0 |
| `lightning_receive_cln` | 12 | 12 | 0 | 0 |
| `lightning_receive_core_ln` | 12 | 12 | 0 | 0 |
| `swap_terms` | 12 | 12 | 0 | 0 |
| `untracked_route` | 12 | 24 | 0 | 0 |
| `liquid_wallet_omitted_scope` | 12 | 12 | 0 | 0 |
| `wallet_scope_conflict` | 12 | 0 | 12 | 0 |
| `conversion_inferred` | 12 | 12 | 0 | 0 |
| `conversion_inferred_chronology` | 12 | 0 | 12 | 0 |
| `conversion_ambiguous` | 12 | 0 | 12 | 0 |
| `native_namespace_gate` | 12 | 0 | 12 | 0 |
| `native_excluded_rows` | 12 | 12 | 0 | 0 |
| `lineage_other_profile` | 12 | 12 | 0 | 0 |
| `authored_source_default` | 12 | 12 | 0 | 0 |
| `retained_reviewed_notes` | 12 | 12 | 0 | 0 |
| `liquid_substring_kind` | 12 | 12 | 0 | 0 |
| `generic_elements_domain` | 12 | 0 | 12 | 0 |
| `fee_null_domain` | 12 | 12 | 0 | 0 |
| `missing_time_empty` | 12 | 0 | 12 | 0 |
| `transaction_id_empty` | 12 | 0 | 12 | 0 |
| `zero_amount_chronology` | 12 | 0 | 12 | 0 |
| `authoritative_chronology` | 12 | 12 | 0 | 0 |
| `asset_upper_semantics` | 12 | 0 | 12 | 0 |
| `asset_hash_upper_019b` | 12 | 12 | 0 | 0 |
| `asset_hash_upper_0264` | 12 | 12 | 0 | 0 |
