# Custody evidence performance checkpoint

Local branch `t3code/custody-perf-evidence`, based on `238422f90`. The changes
are confined to evidence construction, its compatibility boundary, and its
mutation recipes. The **10× Python custody target remains open**.

Canonical observations now hold typed quantities, commitments, authority facts
and borrowed strings. Quantity and detail commitments write their canonical
preimages directly, avoiding redundant JSON and Python object trees. Exceptional
encoding retains the full-document fallback for exact surrogate positions,
recursion refusals and the Python integer-string digit limit.

Enrichment borrows decoded configuration and transfer payloads. Successful
outbound event identities are reused after duplicate fee normalization, which
does not change identity fields. Canonical slots borrow their observations;
an observation-to-selected-leg index replaces repeated alias scans. Snapshot
deduplication borrows commitments, and stage arrays move into compatibility
outputs. Tie selection, last-wins snapshot deduplication, refusal ordering and
output bytes remain unchanged. No dependencies or parallel workers are added.

This is a partial R4-3 implementation. Enrichment, receipt reconciliation and
the compatibility output still use `Value`; the entire evidence stage has not
become a JSON-free typed pipeline. The independently owned component and
projection stages are unchanged.

## Method and profiling

The two committed-seed 20,000-transaction books use the unregistered prepared
R4-13 process harness from the preceding checkpoint. Native R4-10 pre-build
work and registered CLI/daemon routing are outside the measurement. Rust is
1.99.1, Python is locked 3.11, Cargo runs offline with eight build jobs, and
native binaries use release builds with debug=1.

Every comparison contains five alternating pairs on fresh copies of one
migrated book. Process CPU is user plus system time across all threads,
measured using `getrusage(RUSAGE_CHILDREN)`. A scratch-only preload maps native
`Instant` spans to `CLOCK_PROCESS_CPUTIME_ID`; Python uses the existing phase
profiler with `process_time`. Every table retains all spans from each engine's
fastest complete-process trial, rather than combining independent minima.
Nested spans overlap. Output destruction after custody returns remains in
complete-process CPU.

The `perf` executable is absent. A local standard-library `perf_event_open`
sampler records 16,848 generic and 18,639 multi-wallet task-clock IP samples
from the evidence-only baseline helper, with zero lost samples. The period is
1 ms and kernel work is excluded. GDB `pmp.py` captures another 240 stacks per
shape. Unresolved system-library addresses remain unresolved. The diagnostic
helper includes snapshot parsing and output destruction; profiling overhead is
excluded from the paired timings. Allocation, cloning, JSON conversion, event
identity and commitment construction are the main observed costs.

A further probe retained parsed raw payloads between event identity and
commitment construction. Across five alternating evidence-only pairs per
shape, median span CPU regressed from 0.90996 to 0.92805 s generic and 0.95178
to 1.02884 s multi-wallet. That source change was withdrawn. An earlier probe
run accidentally linked a cached library and is excluded from these results.

Raw trials, stage spans, process CPU/wall time, load observations, book hashes,
source and binary hashes, profile summaries and verification records are in
[`performance-evidence.json`](performance-evidence.json). These shared-host
measurements do not establish idle-host acceptance.

## CPU results

Seconds, native baseline against candidate:

| Stage | Generic before | Generic candidate | Multi-wallet before | Multi-wallet candidate |
| --- | ---: | ---: | ---: | ---: |
| Evidence | 1.33994 | 0.89519 | 1.65035 | 1.02505 |
| Custody total | 4.27576 | 3.88259 | 6.96440 | 6.16497 |
| Complete process | 12.21800 | 11.80924 | 15.90001 | 15.17225 |

Evidence improves **1.50× generic and 1.61× multi-wallet**. Custody improves
1.10× and 1.13×. Median evidence CPU is 1.37990/0.99666 s generic and
1.65035/1.02505 s multi-wallet; median custody CPU is 4.54621/4.17030 s and
7.68559/6.28245 s respectively. One-minute load ranges are 25.50–54.13 generic
and 36.62–60.23 multi-wallet.

Seconds, Python against native candidate:

| Stage | Generic Python | Generic native | Multi-wallet Python | Multi-wallet native |
| --- | ---: | ---: | ---: | ---: |
| Evidence | 2.02262 | 1.00164 | 2.24843 | 1.09312 |
| Custody total | 6.00748 | 4.27776 | 16.00967 | 6.51553 |
| Complete process | 14.68807 | 12.49673 | 26.51554 | 16.69800 |

Evidence is **2.02×/2.06× Python**; total custody is **1.40×/2.46×**. Median
evidence CPU is 2.02262/1.01505 s generic and 2.44597/1.14089 s multi-wallet;
median custody CPU is 6.28244/4.58354 s and 17.06189/7.13966 s. One-minute load
ranges are 24.07–35.85 generic and 21.96–32.56 multi-wallet.

All twenty pairs agree on complete scalar-exact database dumps, payloads and
phase sequences. Separate comparisons have different host loads and should
not be combined into a single mixed-minimum result.

## Verification and continuation

All **625 Rust custody tests**, including frozen vectors, pass. Formatting and
Clippy with warnings denied pass. The new exceptional-preimage differential
regression covers Unicode, nonfinite raw values, surrogate errors, recursion
and a 4,301-digit integer.

All **24,000 generated books and 1,030 permutations** pass, including 50,060
byte comparisons and 178 matching refusals. All twelve gap-signature and
eleven interpretation-signature fixtures agree; per-scenario activity and
distinct active-input counts are retained in the JSON record. All **159
recordings and their 159 permutations** pass, with 636 byte comparisons.

The catalog retains all 2,334 inherited IDs, rules and test assignments,
reanchors 22 recipes and adds three, for **2,337 recipes**. All **26 touched-file
recipes** are freshly killed after 19 named unmutated filters pass. Every
mutant must compile with offline `cargo check --locked --lib --tests` before
its assigned tests record a positive failing-test count. Compilation failures
are not kills.

The whole catalog passes: **2,337/2,337 recipes killed**, with no survivors,
compilation failures or runner errors. The eight serial resumable shards kill
269, 292, 292, 292, 292, 292, 292 and 290 remaining recipes, after 429 named
unmutated filter runs pass across those shards. The 26 touched-file kills and
all remaining kills share one source and catalog fingerprint; no earlier kills
are relabelled with a new fingerprint. All eight shard processes exit zero.

The required `classify_tests update` passes without changing the checked-in
inventory. The required CI preflight passes all **57 tests**. The JSON record
retains the final log hashes and the matching source/catalog fingerprints.

Disposable mutation and verification build/source trees are removed after
summarizing the results. Saved benchmark binaries, raw trials, profiles and
small resumable logs/state remain under `target/custody-evidence/`. No test
or background job remains running at this checkpoint.

## Remaining budget

In the Python/native comparison, generic custody still costs 4.27776 s against
the 0.51 s budget, and multi-wallet costs 6.51553 s against 1.21 s. Generic
evidence alone still exceeds the entire generic custody budget. This is not
R4-3/R4-7 acceptance.

Further evidence work must remove legacy enriched-row materialization and
receipt/output conversions while preserving the shared component callers and
every refusal and commitment byte. Shared fact lifetimes need measured CPU
evidence: retaining all parsed payload trees did not help this workload.
In the baseline/candidate comparison, candidate enrichment still costs
0.29077/0.26980 s generic/multi-wallet, receipt reconciliation costs
0.11251/0.18748 s, and canonical construction costs 0.47869/0.55136 s. Its
nested event-construction span falls from 0.68427/0.71196 to
0.34256/0.40719 s. Those spans narrow the remaining work; they overlap and
are not independent additional costs.
The separately owned component and projection improvements must be integrated
before reassessing total custody acceptance.
