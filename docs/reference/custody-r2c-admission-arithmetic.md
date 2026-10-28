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
  `bytes.fromhex`, including whitespace between complete bytes. It does not
  impose a new 32-decoded-byte requirement at this permissive parser boundary.

The frozen graph corpus grows from 297 to **425** witnesses: 108 nonfinite
mapping cases, 16 additional admission/equality cases and four profile merges.
The retained graph mutations move with their helpers. Eleven new admission
entries bring the catalog to **400**; all 25 changed/new entries are killed by
their named tests after five baseline selections pass. The custody Rust suite
passes after the rewrite. Final workspace and Python verification is recorded
with the arithmetic audit below.

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
