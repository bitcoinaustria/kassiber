# R4-9 loader branch and mutation map

This map inventories the native loader in `crates/journals/src/loader.rs` and
its `direct`, `ownership`, `receipts`, `revisions` and `worklist` modules. It
also includes the shared-record/admission and downstream refusal seams changed
by the loader checkpoints. Test modules are witnesses, not loader branches.
The catalogues are `crates/journals/scripts/loader_mutations.json` and
`crates/journals/scripts/loader_blob_mutations.json`; every entry
names its killer in `expect_failing`. A catalogue entry is evidence only after
it builds, its unmutated witness passes, and the mutation fails that witness.
The campaign record, rather than this source inventory, owns kill status.

The tables group alternatives of one decision together. A recipe can pin a
branch's output or downstream effect rather than invert its predicate. `?`
propagates the called operation's error without a new authority or fallback
choice; those edges share that operation's refusal witness. Iterator traversal,
allocation capacity and constructors that make no semantic choice are listed
separately below. SQL ordering and filtering are part of the relevant row-source
contract, even when the Rust call itself has no `if`.

## Scalar helpers and reference errors

| Decision / alternatives | Mutation IDs | Named killers |
| --- | --- | --- |
| SQL NULL / INTEGER / REAL / TEXT / BLOB, preserving INTEGER/REAL identity | `JL-sql-null`, `JL-integer-storage`, `JL-real-storage`, `JL-blob-refusal`; direct variants `JL-direct-null`, `JL-direct-integer`, `JL-direct-real`, `JL-direct-text`, `JL-direct-utf8`, `JL-direct-blob` | `native_loader_keeps_sql_values_and_refuses_blob_coercion`, `native_loader_reads_direct_columns_without_conversion`, `native_loader_scalar_helpers_preserve_types_and_missing_values` |
| Converted NULL, bool, unbounded integer, float, string, list and dict; invalid mapping keys; surrogate strings/keys; Decimal refusal | `JL-convert-null`, `JL-convert-bool`, `JL-convert-int`, `JL-convert-list`, `JL-convert-key`, `JL-convert-surrogate-string`, `JL-convert-surrogate-key`, `JL-convert-decimal`, `JL-direct-nonfinite`, `JL-negative-nonfinite` | `native_loader_scalar_helpers_preserve_types_and_missing_values`, `native_loader_preserves_nonfinite_real_until_downstream` |
| Finite float spelling, both infinity signs, impossible SQLite NaN | `JL-direct-real`, `JL-direct-nonfinite`, `JL-negative-nonfinite`, `JL-nan-storage` | `native_loader_reads_direct_columns_without_conversion`, `native_loader_preserves_nonfinite_real_until_downstream` |
| Mapping lookup found / missing / non-mapping; set replaces an existing value / appends a new field | `JL-get-found`, `JL-set-existing`, `JL-set-new` | `native_loader_scalar_helpers_preserve_types_and_missing_values` |
| Truth of NULL, bool, zero/nonzero integer and float, empty/nonempty string/list/dict/Decimal | `JL-truth-null`, `JL-truth-bool`, `JL-truth-int`, `JL-truth-float`, `JL-truth-string`, `JL-truth-list`, `JL-truth-dict`, `JL-truth-decimal` | `native_loader_scalar_helpers_preserve_types_and_missing_values` |
| Parsed string / invalid JSON / non-string fallback | `JL-parsed-string` | `native_loader_scalar_helpers_preserve_types_and_missing_values` |
| AppError family, message, optional details/hint, code and retryability | `JL-app-family`, `JL-app-code`, `JL-app-hint`, `JL-app-details`, `JL-app-retryable` | `native_loader_refusal_attributes_and_display_keep_private_values_out` |
| Redacted database/invalid/reference errors and public static display alternatives | `JL-display-database`, `JL-display-invalid`, `JL-display-reference` | `native_loader_refusal_attributes_and_display_keep_private_values_out` |
| Operational SQLite failure / input failure / UTF-8 OperationalError versus other errors | `JL-operational-class`, `JL-utf8-review-fallback` | `native_loader_refusal_attributes_and_display_keep_private_values_out`, `native_loader_utf8_optional_reviews_preserve_python_fallback` |
| UTF-8 valid / invalid; physical column precedence; duplicate or discarded TEXT still decoded; NUL, non-ASCII and truncation rendering | `JL-direct-utf8`, `JL-utf8-column`, `JL-utf8-limit`, `JL-utf8-nul`, `JL-utf8-ascii`, `JL-utf8-physical-order`, `JL-utf8-auxiliary-validation`, `JL-query-discarded-text`, `JL-dropped-text-validation` | `native_loader_utf8_refusals_preserve_python_column_order_and_message`, `native_loader_filters_only_safe_confirmation_columns`, `native_loader_reads_direct_columns_without_conversion` |
| Duplicate SQL names use the first value; ordinary and direct reader paths | `JL-duplicate-column`, `JL-direct-duplicate` | `native_loader_keeps_sql_values_and_refuses_blob_coercion`, `native_loader_reads_direct_columns_without_conversion` |

## Safe configuration and direct records

| Decision / alternatives | Mutation IDs | Named killers |
| --- | --- | --- |
| Safe versus raw `_closed_confirmation_observation`, including validation before dropping TEXT | `JL-safe-confirmation-column`, `JL-raw-confirmation-column`, `JL-safe-row-confirmation`, `JL-dropped-text-validation` | `native_loader_filters_only_safe_confirmation_columns` |
| Present config fields versus absent fields; safe versus raw row; config versus wallet config marker | `JL-config-marker`, `JL-config-direct-marker`, `JL-existing-config-marker`, `JL-config-second-column`, `JL-raw-confirmation-column` | `native_loader_matches_python_database_bytes`, `native_loader_reads_direct_columns_without_conversion`, `native_loader_filters_only_safe_confirmation_columns`, `shared_snapshot_config_admission_checks_every_population` |
| String JSON versus object/scalar config; mapping versus non-mapping; public fact allowlist | `JL-config-nontext`, `JL-nonobject-config`, `JL-config-object-raw`, `JL-config-allowlist` | `native_loader_reads_direct_columns_without_conversion`, `native_loader_matches_python_database_bytes`, `native_loader_adapter_preserves_order_and_refuses_invalid_shapes` |
| Samourai absent / explicit null / object / scalar; only role/group/section retained | `JL-config-samourai-fields`, `JL-config-samourai-shape`, `JL-config-allowlist` | `native_loader_adapter_preserves_order_and_refuses_invalid_shapes` |
| Cold versus warm config memo; distinct configs and columns remain checked | `JL-config-cache-hit`, `JL-config-cache-insert`, `JL-config-cache-value`, `JL-config-memo-admission`, `JL-config-second-column`, `JL-config-anchor-admission` | `native_loader_reads_direct_columns_without_conversion`, `native_loader_bounds_gap_worklist`, `shared_snapshot_config_admission_checks_every_population` |
| Shared columns sorted/unique, shared layout, get/contains/map/serialize position | `JL-shared-column-order`, `JL-shared-column-unique`, `JL-shared-column-reuse`, `JL-shared-get-position`, `JL-shared-contains`, `JL-shared-map-values`, `JL-shared-serialize` | `shared_record_preserves_map_semantics` |
| Native rows must be a list of objects; present versus missing string values | `JL-adapter-list-shape`, `JL-adapter-row-shape`, `JL-adapter-missing-string` | `native_loader_adapter_preserves_order_and_refuses_invalid_shapes` |
| V1 ordered override versus native value; arrays, objects and scalars; SQL columns versus fixed root/network/component/worklist fields versus fallback object order; anchor wildcard versus root/nested paths | `JL-column-order`, `JL-gate-ordered`, `JL-gate-native-order`, `JL-gate-anchor-path`, `JL-gate-fixed-order`, `JL-gate-root-path` | `native_loader_reads_direct_columns_without_conversion`, `native_loader_matches_python_database_bytes`, `native_loader_adapter_preserves_order_and_refuses_invalid_shapes` |
| Both direct consumers use database admission; transport retains strict signed-64 quantity admission | `JL-loaded-admission`, `JL-projection-loaded-admission` | `native_loader_legacy_scalars_keep_downstream_admission` |

## Snapshot composition, ordering and refusal precedence

| Decision / alternatives | Mutation IDs | Named killers |
| --- | --- | --- |
| Autocommit, deferred/read transaction versus caller write transaction; database refusal preserves caller rollback | `JL-write-lock` | `native_loader_requires_caller_write_transaction` |
| Active count and transaction/wallet/profile selection; all rows versus bounded >50k worklist | `JL-direct-active-count`, `JL-direct-threshold`, `JL-worklist-threshold` | `native_loader_selects_bounded_worklist_from_active_count`, `native_loader_bounds_gap_worklist` |
| Current versus stale/null authority; graph truth/equality; amount before fee; truthy quantity conversion including overflow | `JL-confirmation-capture`, `JL-confirmation-null`, `JL-confirmation-version`, `JL-confirmation-graph`, `JL-confirmation-field-order`, `JL-confirmation-truth`, `JL-confirmation-overflow` | `native_loader_preserves_receipt_error_precedence`, `native_loader_integer_overflow_refusals_match_python`, `native_loader_matches_python_database_bytes` |
| Active components, leg ordering, anchor identity/lookup, anchor wallets, referenced revision identity, non-active revision headers | `JL-active-components`, `JL-leg-order`, `JL-anchor-lookup`, `JL-anchor-identity`, `JL-anchor-wallet`, `JL-revision-identity`, `JL-revision-bind` | `native_loader_matches_python_database_bytes`, `native_loader_revision_queries_keep_sqlite_binding_types` |
| Mixed TEXT/numeric/BLOB revision populations refuse before queries/output with one typed validation error | `JL-revision-reference-storage`, `JL-revision-message-family`, `JL-revision-message-text`, `JL-revision-message-int`, `JL-revision-message-real`, `JL-revision-validation-code`, `JL-reference-mixed-type-guard` | `native_loader_refuses_mixed_revision_references_before_output`, `native_loader_revision_order_preserves_python_numeric_identity`, `test_loader_mixed_revision_refusal_is_stable_across_hash_seeds` |
| Modern rates versus operational legacy-column fallback; optional reviews versus empty operational fallback; latest revision and stable review order | `JL-rate-fallback`, `JL-review-fallback`, `JL-latest-review`, `JL-utf8-review-fallback` | `native_loader_matches_python_database_bytes`, `native_loader_utf8_optional_reviews_preserve_python_fallback` |
| Channel records only; live loan legs; custody-component evidence; unresolved migration issues; pair dismissals and ordered auxiliary rows | `JL-channel-kind`, `JL-loan-deletion`, `JL-evidence-subject`, `JL-migration-resolution` | `native_loader_matches_python_database_bytes` |
| Receipt load succeeds / refuses; earlier accounting/component error wins; builtin exception family versus AppError code | `JL-receipt-precedence`, `JL-receipt-error-family`, `JL-receipt-accounting-precedence`, `JL-receipt-component-precedence` | `native_loader_preserves_receipt_error_precedence` |
| Network table absent / present; optional reference tables or required scope columns absent / present; policy private/source presence | `JL-network-table`, `JL-network-reference-table`, `JL-network-reference-columns`, `JL-network-private`, `JL-network-source` | `native_loader_matches_python_database_bytes` |

## Ownership

| Decision / alternatives | Mutation IDs | Named killers |
| --- | --- | --- |
| Ownership inactive without observed authority versus loading inventory/current/retired sources | `JL-owned-activation`, `JL-retired-policy`, `JL-owned-history-nonempty` | `native_loader_matches_python_database_bytes` |
| Group insertion retains first-key order and all evidence; missing lookup; reference filtering and scoped/unscoped deterministic sorting | `JL-index-reference-filter`, `JL-index-scoped-filter`, `JL-index-txid-sort`, `JL-script-source`, `JL-outpoint-source` | `native_loader_ownership_helpers_keep_scope_and_evidence_separate`, `native_loader_matches_python_database_bytes` |
| Empty/truthy address, bech32 lowercasing versus base58 case retention | `JL-address-case`, `JL-address-base58`, `JL-index-address-empty` | `native_loader_ownership_helpers_keep_scope_and_evidence_separate` |
| Int, bool, finite float truncation, numeric text versus unsupported/malformed/nonfinite conversion | `JL-integer-int`, `JL-integer-bool`, `JL-integer-float`, `JL-integer-string`, `JL-integer-overflow` | `native_loader_ownership_helpers_keep_scope_and_evidence_separate`, `native_loader_caught_integer_errors_still_propagate_overflow` |
| Outpoint int conversion: string parsing, digit-limit prefix/sign/ASCII or Unicode whitespace/invalid suffix, repr limit, NaN/infinity, type errors | `JL-owned-outpoint-refusal`, `JL-outpoint-repr-limit`, `JL-outpoint-digit-limit`, `JL-outpoint-nonfinite`, `JL-outpoint-nan-family`, `JL-outpoint-infinity-family`, `JL-outpoint-prefix-limit`, `JL-outpoint-prefix-sign`, `JL-outpoint-prefix-unicode-space`, `JL-outpoint-prefix-parse` | `native_loader_preserves_receipt_error_precedence`, `native_loader_outpoint_integer_preserves_python_refusals` |
| Empty/odd/nonhex script versus valid script; permitted whitespace and case normalization | `JL-receipt-empty-script-fallback`, `JL-script-space`, `JL-script-hex`, `JL-index-script-case`, `JL-index-script-empty` | `native_loader_matches_python_database_bytes`, `native_loader_ownership_helpers_keep_scope_and_evidence_separate` |
| Missing/malformed config falls back; decoded non-mapping config refuses; unordered summary query defines first refusal | `JL-owned-config-refusal-family`, `JL-owned-config-refusal-message`, `JL-owned-summary-refusal-order` | `native_loader_preserves_receipt_error_precedence` |
| Address NULL / comma-separated string / list / dict keys / non-iterable refusal; stripping, empties and script/text identity deduplication | `JL-addresses-string`, `JL-addresses-list`, `JL-addresses-dict`, `JL-addresses-empty`, `JL-addresses-dedupe`, `JL-owned-address-refusal-family`, `JL-owned-address-refusal-message` | `native_loader_ownership_helpers_keep_scope_and_evidence_separate`, `native_loader_preserves_receipt_error_precedence` |
| Default chain/network, recognized aliases versus literal normalized fallback; descriptor versus xpub+script-types and empty descriptors | `JL-chain-default`, `JL-scope-fallback`, `JL-descriptor-presence`, `JL-descriptor-empty` | `native_loader_ownership_helpers_keep_scope_and_evidence_separate` |
| Absent txid/vout does not create outpoint; lowercased identities; scoped evidence and deduplicated sorted wallet pairs | `JL-index-outpoint-absent`, `JL-index-outpoint-case`, `JL-index-txid-empty`, `JL-index-txid-dedupe`, `JL-index-scoped-dedupe`, `JL-index-scoped-filter`, `JL-index-txid-sort` | `native_loader_ownership_helpers_keep_scope_and_evidence_separate` |
| Inventory wallet must belong to profile; no-wallet fallback; empty label fallback; inventory has empty account; row chain/network override config; address script precedes stored script | `JL-owned-wallet-scope`, `JL-owned-empty-inventory-label`, `JL-inventory-account`, `JL-owned-inventory-chain`, `JL-owned-inventory-network`, `JL-owned-inventory-script` | `native_loader_matches_python_database_bytes` |
| Valid nonnegative inventory high-water, scan default/minimum/cap, invalid scan warning; retired mapping policy and address/derived source labels | `JL-owned-used-horizon`, `JL-owned-current-scan`, `JL-owned-scan-cap`, `JL-scan-warning`, `JL-owned-history-nonempty`, `JL-owned-history-source`, `JL-derived-history`, `JL-owned-derived-source` | `native_loader_matches_python_database_bytes` |
| Descriptor absent / valid / invalid; current/historic floor, invalid-floor warning, int overflow, used index plus gap, bounded horizon, memo by descriptor+horizon, derivation and unconfidential alias | `JL-owned-descriptor-gate`, `JL-owned-floor-warning`, `JL-current-scan-overflow`, `JL-historic-scan-overflow`, `JL-owned-used-horizon`, `JL-owned-cache-horizon`, `JL-owned-cache-descriptor`, `JL-owned-derive-start`, `JL-owned-unconfidential` | `native_loader_matches_python_database_bytes`, `native_loader_preserves_receipt_error_precedence` |
| Legacy inferred outpoint requires canonical hex txid, list of matching observations, nonempty address, non-bool/nonnegative vout and exclusive wallet ownership | `JL-owned-infer-txid`, `JL-owned-infer-observed`, `JL-owned-infer-vout`, `JL-owned-infer-negative`, `JL-owned-infer-exclusive` | `native_loader_matches_python_database_bytes` |
| References from parsed graph inputs/outputs/scripts/outpoints/txid; unrelated sources culled | `JL-owned-reference-inputs`, `JL-owned-reference-txid`, `JL-index-reference-filter`, `JL-index-scoped-filter` | `native_loader_matches_python_database_bytes`, `native_loader_ownership_helpers_keep_scope_and_evidence_separate` |
| Scoped txid wallet pairs sort by wallet identity; scoped outpoint evidence retains source order | `JL-index-scoped-sort`, `JL-index-scoped-outpoint-order` | `native_loader_scoped_index_preserves_evidence_order_and_sorts_wallet_pairs` |

## Receipt sources, revisions and worklist

| Decision / alternatives | Mutation IDs | Named killers |
| --- | --- | --- |
| Receipt config mapping versus malformed/scalar fallback; explicit missing chain/network must remain unscoped; address-list versus inventory default label/kind | `JL-receipt-config-fallback`, `JL-receipt-scope-missing`, `JL-receipt-kind-fallback`, `JL-receipt-label-fallback`, `JL-receipt-raw-inventory-label` | `native_loader_receipt_defaults_preserve_missing_scope_and_source_facts`, `native_loader_matches_python_database_bytes` |
| Lexical possible-identity filter: external string/number; consecutive hex length/reset; escaped JSON; object/list; impossible shapes | `JL-receipt-lexical-admission`, `JL-receipt-escaped-json`, `JL-receipt-identity-length`, `JL-receipt-identity-reset`, `JL-receipt-numeric-id`, `JL-receipt-object-graph` | `native_loader_matches_python_database_bytes`, `native_loader_receipt_filter_never_rejects_possible_identities` |
| Bitcoin/BTC scope and inbound wallets; multiple distinct recipients versus early empty candidate | `JL-receipt-scope-chain`, `JL-receipt-scope-asset`, `JL-receipt-wallet-count`, `JL-receipt-wallet-direction`, `JL-receipt-empty-candidate` | `native_loader_matches_python_database_bytes` |
| Scoped address lists; valid addresses only; active transaction counts/deprecated facts; inventory direct/raw/address script precedence and wallet fallback | `JL-receipt-scope-empty`, `JL-receipt-address-label`, `JL-receipt-script-raw`, `JL-receipt-script-address`, `JL-receipt-count`, `JL-receipt-deprecated`, `JL-receipt-raw-inventory-label` | `native_loader_matches_python_database_bytes` |
| Inventory branch/index MAX; checkpoint highest-used mapping and parsed integer keys; alias/duplicate last wins before max merge; negative floor; descriptor absent/invalid/valid; bounded default or known-index horizon; matching scripts only | `JL-receipt-branch-overflow`, `JL-receipt-index-overflow`, `JL-checkpoint-overflow`, `JL-receipt-alias-last`, `JL-receipt-checkpoint-merge`, `JL-receipt-negative-floor`, `JL-receipt-default-horizon`, `JL-receipt-horizon`, `JL-receipt-derive-start`, `JL-receipt-script-filter` | `native_loader_integer_overflow_refusals_match_python`, `native_loader_preserves_receipt_error_precedence`, `native_loader_matches_python_database_bytes` |
| Revision scalar string/number/unsupported; integer versus real identity; same/mixed numeric comparisons, range endpoints, fraction, integer identity above 2^53, text ordering and original binding | `JL-revision-unsupported`, `JL-revision-is-text`, `JL-revision-integer-order`, `JL-revision-real-order`, `JL-revision-mixed-integer-real`, `JL-revision-mixed-real-integer`, `JL-revision-upper-range`, `JL-revision-lower-range`, `JL-revision-fraction`, `JL-revision-exact-integer`, `JL-revision-text-order`, `JL-revision-bind` | `native_loader_revision_order_preserves_python_numeric_identity`, `native_loader_revision_queries_keep_sqlite_binding_types` |
| Worklist <=50k versus bounded population; structured source limit, remaining ordinary budget, global cross-asset amount/date/id order, empty/nonempty return ranges, return limit and final stable order | `JL-worklist-threshold`, `JL-typed-limit`, `JL-source-budget`, `JL-worklist-budget-branch`, `JL-worklist-source-order`, `JL-worklist-range-branch`, `JL-return-limit` | `native_loader_bounds_gap_worklist`, `native_loader_worklist_ranks_sources_across_assets_before_budgeting` |
| Worklist SQL NULL/int/real/text binding versus out-of-range/surrogate/unsupported values; truthy string versus empty fallback | `JL-parameter-null`, `JL-parameter-int`, `JL-parameter-real`, `JL-parameter-text`, `JL-parameter-refusal`, `JL-worklist-string` | `native_loader_worklist_parameters_keep_sqlite_scalar_types` |
| Consumer component integer/timestamp and finalized price refusal messages/codes; refusals require exact complete code/message pairs across seeds and cannot admit a refused projection | `JL-component-integer-infinity`, `JL-component-integer-message`, `JL-component-sort-message`, `JL-price-message`, `JL-price-code`, `JL-audit-seed-variants`, `JL-audit-seed-code` | `component_integer_refusal_messages_match_python`, `component_header_sort_retains_python_type_errors`, `finalized_price_errors_keep_field_code_and_message`, `test_loader_revision_audit_requires_exact_refusal_pairs` |

## Individual SQL scope guards

Each static query source below also has its own mutation. These negate the
requested identity comparison while preserving the complete bind list.

| Mutation ID | Module / source table | Named killer |
| --- | --- | --- |
| `JL-sql-loader-scope-01` | `loader` / `sqlite_master` | `native_loader_matches_python_database_bytes` |
| `JL-sql-loader-scope-02` | `loader` / `transactions` | `native_loader_matches_python_database_bytes` |
| `JL-sql-direct-scope-01` | `direct` / `transactions` | `native_loader_selects_bounded_worklist_from_active_count` |
| `JL-sql-direct-scope-02` | `direct` / `wallets` | `native_loader_matches_python_database_bytes` |
| `JL-sql-direct-scope-03` | `direct` / `custody_components` | `native_loader_matches_python_database_bytes` |
| `JL-sql-direct-scope-04` | `direct` / `custody_component_legs` | `native_loader_matches_python_database_bytes` |
| `JL-sql-direct-scope-05` | `direct` / `custody_component_allocations` | `native_loader_matches_python_database_bytes` |
| `JL-sql-direct-scope-06` | `direct` / `custody_component_economic_terms` | `native_loader_matches_python_database_bytes` |
| `JL-sql-direct-scope-07` | `direct` / `custody_component_evidence_commitments` | `native_loader_matches_python_database_bytes` |
| `JL-sql-direct-scope-08` | `direct` / `transactions` | `native_loader_matches_python_database_bytes` |
| `JL-sql-direct-scope-09` | `direct` / `wallets` | `native_loader_matches_python_database_bytes` |
| `JL-sql-direct-scope-10` | `direct` / `custody_components` | `native_loader_matches_python_database_bytes` |
| `JL-sql-direct-scope-11` | `direct` / `lightning_node_records` | `native_loader_matches_python_database_bytes` |
| `JL-sql-direct-scope-12` | `direct` / `transactions` | `native_loader_matches_python_database_bytes` |
| `JL-sql-direct-scope-13` | `direct` / `transactions` | `native_loader_matches_python_database_bytes` |
| `JL-sql-direct-scope-14` | `direct` / `transactions` | `native_loader_matches_python_database_bytes` |
| `JL-sql-direct-scope-15` | `direct` / `custody_gap_reviews` | `native_loader_matches_python_database_bytes` |
| `JL-sql-direct-scope-16` | `direct` / `loan_legs` | `native_loader_matches_python_database_bytes` |
| `JL-sql-direct-scope-17` | `direct` / `custody_authored_evidence_snapshots` | `native_loader_matches_python_database_bytes` |
| `JL-sql-direct-scope-18` | `direct` / `custody_authored_migration_issues` | `native_loader_matches_python_database_bytes` |
| `JL-sql-direct-scope-19` | `direct` / `transaction_pair_dismissals` | `native_loader_matches_python_database_bytes` |
| `JL-sql-direct-scope-20` | `direct` / `book_network_bindings` | `native_loader_matches_python_database_bytes` |
| `JL-sql-direct-scope-21` | `direct` / `transactions` | `native_loader_matches_python_database_bytes` |
| `JL-sql-direct-scope-22` | `direct` / `formatted query` | `native_loader_matches_python_database_bytes` |
| `JL-sql-direct-scope-23` | `direct` / `wallet_policy_sources` | `native_loader_matches_python_database_bytes` |
| `JL-sql-ownership-scope-01` | `ownership` / `wallets` | `native_loader_matches_python_database_bytes` |
| `JL-sql-ownership-scope-02` | `ownership` / `wallets` | `native_loader_matches_python_database_bytes` |
| `JL-sql-ownership-scope-03` | `ownership` / `wallet_utxos` | `native_loader_matches_python_database_bytes` |
| `JL-sql-ownership-scope-04` | `ownership` / `wallet_policy_epochs` | `native_loader_matches_python_database_bytes` |
| `JL-sql-ownership-scope-05` | `ownership` / `transactions` | `native_loader_matches_python_database_bytes` |
| `JL-sql-receipts-scope-01` | `receipts` / `transactions` | `native_loader_matches_python_database_bytes` |
| `JL-sql-receipts-scope-02` | `receipts` / `wallets` | `native_loader_matches_python_database_bytes` |
| `JL-sql-receipts-scope-03` | `receipts` / `wallet_utxos` | `native_loader_matches_python_database_bytes` |
| `JL-sql-receipts-scope-04` | `receipts` / `wallet_utxos` | `native_loader_matches_python_database_bytes` |
| `JL-sql-receipts-scope-05` | `receipts` / `freshness_source_states` | `native_loader_matches_python_database_bytes` |
| `JL-sql-worklist-scope-01` | `worklist` / `transactions` | `native_loader_bounds_gap_worklist` |
| `JL-sql-worklist-scope-02` | `worklist` / `wallets` | `native_loader_bounds_gap_worklist` |
| `JL-sql-worklist-scope-03` | `worklist` / `transactions` | `native_loader_bounds_gap_worklist` |

## Delegation and defensive paths

`From<rusqlite::Error>`, `dict`, `list`, `text`, `safe_rows`, `normalize`,
`match_facts`, `wallet_pair`, `PartialOrd`, `PartialEq`, `ToSql`, and the memo-drop
RAII guard add no independent accounting choice. Their results feed the mapped
query, normalization, index, revision and refusal witnesses. `floor_queries`
returns benchmark SQL only and is not called by the loader.

SQLite cannot produce REAL NaN (binding stores NULL), JSON number parse errors
from formatted SQL integers/floats, Decimal SQL values, non-string JSON mapping
keys, a surrogate SQL TEXT string, or a non-object `query` row. These are
representation defenses rather than accepted alternative storage contracts.
The scalar and adapter witnesses exercise the helper-reachable defenses.
`unreachable!` cases require violating private constructor invariants (a
non-mapping record passed to `set`, a non-pair passed to `wallet_pair`, or a
non-scalar `Reference`); no database or public loader input can construct them.
Descriptor target missing-field fallbacks and errors propagate the existing
wallet descriptor module's typed result contract. A clamped derivation horizon
cannot overflow `u32`; worklist range conversion errors retain their fail-closed
conversion rather than coerce an out-of-range amount.

The legacy inference range recipe inverts the nonnegative predicate and is
killed by an admitted, referenced positive outpoint. Merely removing the
negative guard can be observationally equivalent after graph-reference culling:
invalid negative graph outpoints cannot retain that inferred evidence. The map
therefore pins the range decision without claiming that the earlier exploratory
guard-removal recipe exposed an unsafe admission. Reader capacity and vector
allocation hints change allocation only; they add no projection alternative.

The original loader-9 record retained the BLOB corruption class and sixteen
seed-dependent Python messages as separate pending exceptions. The subsequent
accepted BLOB continuation closes the former; forty measured messages remain
pending the Python fix. The loader-9 campaign kills all **308 loader
recipes**, after all **33 unmutated witness filters** pass, with no survivors or
runner errors. The separate custody campaign's final kill set equals all
**2,292 entries**. Its original run kills 2,289; three recipes are re-anchored
to preserve their stated contracts and killed in fresh shards. The
[acceptance record](r4-native-loader-checkpoint.md#loader-9-acceptance) owns
provenance, the re-anchor review and the remaining gates.

## Integrated BLOB contracts

The combined catalogue has 357 entries: the 308 loader-9 recipes, 47 BLOB
recipes and two deterministic-refusal recipes. No entry is removed.
The BLOB continuation adds the following named recipes to the branch map.
Byte identity, literal-map escaping, decoding and conversion remain consumer
contracts; they do not grant ownership or accounting authority. Mixed revision
references now share the same validation refusal as the port-branch Python
reference. INTEGER and REAL remain one sortable family. `Reference::python_type`
now exists only in the scalar identity witnesses; the
retained `JL-revision-message-int`/`real` recipes pin those witness types rather
than select the runtime refusal. The historical
`JL-audit-seed-variants` and `JB-audit-blob-variant` IDs are retained but their
recipes now enforce exact matching and BLOB seed-sweep coverage.

| Module / seam | Mutation IDs | Named killers |
| --- | --- | --- |
| `crates/custody/src/codec.rs` | `JB-typed-from-json`, `JB-typed-to-json`, `JB-bytes-truth`, `JB-bytes-string`, `JB-bytes-integer`, `JB-bytes-fast-json`, `JB-literal-tag-escape`, `JB-literal-map-roundtrip`, `JB-json-bytes-object` | `raw_bytes_preserve_identity_truth_integer_and_literal_maps`, `json_snapshot_cannot_forge_internal_byte_scalars` |
| `crates/custody/src/components/mod.rs` | `JB-component-int-kind`, `JB-component-int-repr`, `JB-component-byte-sort`, `JB-component-header-bytes` | `component_bytes_sort_and_integer_semantics_match_python` |
| `crates/custody/src/evidence.rs` | `JB-evidence-byte-policy` | `evidence_bytes_use_utf8_replacement_before_canonical_json` |
| `crates/custody/src/graph/stored.rs` | `JB-graph-byte-object`, `JB-graph-byte-parse`, `JB-graph-byte-truth` | `stored_graph_json_bytes_decode_only_at_the_mapping_consumer` |
| `crates/custody/src/lib.rs` | `JB-snapshot-wire-tag-guard` | `json_snapshot_cannot_forge_internal_byte_scalars` |
| `crates/journals/scripts/audit_loader_storage.py` | `JB-audit-blob-variant` | `tests/test_rust_custody_loader.py::test_loader_revision_audit_requires_exact_refusal_pairs` |
| `crates/journals/src/loader.rs` | `JB-config-memo`, `JB-parsed-bytes`, `JB-convert-literal-map`, `JB-query-bytes-truth` | `native_loader_blob_conversions_preserve_bytes_and_consuming_policy`, `native_loader_reads_direct_columns_without_conversion` |
| `crates/journals/src/loader/direct.rs` | `JB-revision-mixed-text`, `JB-revision-mixed-number`, `JB-gate-literal-map` | `native_loader_blob_revision_references_bind_and_refuse_before_output`, `native_loader_refuses_mixed_blob_numeric_revisions_before_queries`, `native_loader_reads_direct_columns_without_conversion` |
| `crates/journals/src/loader/ownership.rs` | `JB-ownership-config`, `JB-bytes-int-ascii`, `JB-bytes-int-error`, `JB-outpoint-bytes-ascii` | `native_loader_blob_conversions_preserve_bytes_and_consuming_policy` |
| `crates/journals/src/loader/revisions.rs` | `JB-revision-bind`, `JB-revision-byte-order`, `JB-revision-byte-type` | `native_loader_blob_revision_references_bind_and_refuse_before_output`, `native_loader_blob_revision_identity_is_distinct_and_lexicographic` |
| `crates/pyjson/src/bytes.rs` | `JB-json-bytes-bom`, `JB-json-bytes-surrogates`, `JB-json-bytes-utf32-le`, `JB-json-bytes-utf32-be`, `JB-json-bytes-utf16-le`, `JB-json-bytes-utf16-be`, `JB-json-bytes-truncation`, `JB-json-bytes-unpaired`, `JB-json-bytes-short-utf16` | `json_bytes_detect_encoding_without_changing_bytes_identity`, `raw_bytes_preserve_identity_truth_integer_and_literal_maps` |
| `crates/pyjson/src/encode.rs` | `JB-json-bytes-refusal`, `JB-json-bytes-default`, `JB-json-bytes-message` | `bytes_json_policy_and_repr_match_python`, `native_loader_blob_conversions_preserve_bytes_and_consuming_policy` |
| `crates/pyjson/src/value.rs` | `JB-bytes-repr-quotes` | `bytes_json_policy_and_repr_match_python` |
| `crates/pytext/src/json.rs` | `JB-json-raw-surrogates` | `json_parser_preserves_raw_surrogate_identity` |

> inherited from t3code/r5-transfers-17@59e1476a4; combined R10 results are in the [R10 audit](rust-backend-r10-integration.md).
## Transfer-branch integration

Transfers-14 retains this original map and its campaign provenance. Raw database
admission now lives in `crates/custody/src/gates/loaded.rs` beside typed admission;
downstream stages are shared. The review-build witness compares both paths.
The BLOB catalogue additionally contains `JB-shared-snapshot-tag-cache`, killed
by `shared_snapshot_wire_tags_reset_cached_rows`, to prevent a warmed transaction
view from bypassing literal-tag sanitization. Finalized-output guards
`JB-projection-byte-move` and `JB-projection-literal-map-move` pin database byte
identity and literal JSON mapping identity through the typed assembler's
move-based output conversion. The loader catalogue retains its 308 original
recipes and adds two guards for the
review assembler's inherited strict admission and bounded refusal. The
[integration handoff](rust-shell.md#accepted-loader-integration-r5-transfers-14)
distinguishes fresh checks from inherited campaign evidence.
