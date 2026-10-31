# R4-9 native custody loader checkpoint

This is an **unaccepted, test-only checkpoint**, updated 2026-10-31. Loader-6
continues `c60e0cb9f` (loader-5), preserves SQLite infinities and numeric revision
bindings, and audits native finalized projection separately from v1 transport.
Its storage and downstream refusal gaps still block acceptance. Python retains
every production load and journal route. The loader-3/4/5 sections below retain
their historical evidence.

Final loader-4 measurements retain the 0.30 s target at **0.290 s process CPU /
0.293 s wall**, including admission. Multi-wallet 20k measures **0.439 / 0.444 s**.
Both CPU measurements remain within 5% of loader-3. The historical loader-3
timing and verification sections below retain that run's evidence; they do not
imply fresh reruns of its complete gates.
Exhaustive loader mutations, lossless legacy values/refusals and the complete
custody campaign still keep R4-9 open.

## Representation and authority

`crates/journals/src/loader.rs` exposes `load(connection, profile, as_of)` only
under tests or `test-harness`. The connection belongs to an already opened
`kassiber_store::Book`. Loading requires the caller's SQLite write transaction,
reads pending writes, and never begins, commits, rolls back, repairs or updates
the book. Existing canary tests preserve caller rollback ownership on success
and refusal. No product, daemon or AI route registers the loader probe.

SQL rows populate custody records directly. A record now holds a shared,
sorted, immutable `Columns` layout plus a vector of values. Query preparation
allocates field names once; rows clone only the `Arc` handle. Name lookup,
iteration, missing-field behavior, duplicate JSON keys and serde object output
preserve the former `serde_json::Map` contract. Explicit `into_map()` calls
materialize compatibility objects where existing stages still need JSON.
The JSON adapter and the native loader use the same record type. Rust report
and transfer snapshot builders also adapt to this shared contract.

The integrated typed-row checkpoint retains its direct `BookSnapshot` admission
and the independent complete-v1 byte gate. See the [R8 integration audit](rust-backend-r8-integration.md#combined-performance)
for the prior combined measurements; the following source-line timings precede
this integration and do not establish new combined acceptance.

The generic-20k transactions, rate and gap projections previously owned
1.70 million field keys across 60,000 maps. Those projections now share three
column layouts. Network observations still materialize JSON objects for the
existing network consumer. This is a transitional row representation, not the
plan's eventual lossless `SqlValue`/`BookSnapshot` boundary. INTEGER and REAL
remain distinct, and BLOB, invalid UTF-8 TEXT and nonfinite REAL still have
unfinished legacy/refusal fidelity.

Admission memoizes only successfully checked, identical configuration strings
within one validation call. Every different string and every object still goes
through the same configuration allowlist. Wallets, transactions, anchor wallets,
channels, gap rows and anchor objects retain both configuration checks, and
transaction principal/fee admission still requires exact signed-64 msat.
No memo survives a snapshot validation or grants accounting authority.

Ownership, descriptor derivation, receipt sources and the bounded >50k worklist
retain the loader-2 implementation except for the loader-4 corrections below.
Column-order metadata and ordered derived facts remain solely for the v1 byte
gate. The loader adds no egress, dependency, durable storage or production
runtime requirement. `Cargo.lock` is unchanged.

## Loader-3 paired timing and profile

[`r4-native-loader-3-bench.json`](r4-native-loader-3-bench.json) contains all
20 pairs, generator manifests, byte lengths/hashes, five repetitions of the
four-query SQL floor, the release binary hash and sampling summaries.
The disposable books are copies of loader-2's committed-seed generator books:
generic seed 1 and multi-wallet seed 7, at 2k/20k. All four first pass the
complete Python/native byte gate; SQLCipher parity is checked separately.

`scripts/benchmark-custody-loader.py` alternates Python/native ordering for
five back-to-back pairs per book. CPU is process user+sys. Each implementation
selects its best complete CPU sample; wall time comes from that same sample.
Native total includes admission on the directly built snapshot. Book open,
BEGIN and the test-only serializer are excluded. No snapshot conversion is
moved outside the timer: consumers receive the shared records directly.

| Book | Python CPU / wall | Native load CPU / wall | Admission CPU / wall | Native total CPU / wall | 1-minute load |
| --- | ---: | ---: | ---: | ---: | ---: |
| generic 2,000 | 0.113 / 0.114 | 0.026 / 0.026 | 0.001 / 0.001 | **0.027 / 0.027** | 11.75–11.75 |
| multi-wallet 2,000 | 0.150 / 0.151 | 0.051 / 0.052 | 0.002 / 0.002 | **0.053 / 0.054** | 11.53–11.53 |
| generic 20,000 | 1.168 / 1.177 | 0.280 / 0.282 | 0.009 / 0.009 | **0.289 / 0.291** | 11.22–11.41 |
| multi-wallet 20,000 | 1.443 / 1.454 | 0.406 / 0.409 | 0.023 / 0.024 | **0.429 / 0.433** | 11.10–11.17 |

Compared with the inherited loader-2 measurements, generic20k CPU falls from
0.505 to 0.289 s (43%) and multi-wallet20k from 0.687 to 0.429 s (37%). These
are separate shared-host runs, not controlled before/after pairs. The current
Python/native comparisons above are paired. The four-query SQL floor is
0.073 s CPU / 0.073 s wall at generic20k and 0.090 / 0.090 s at multi-wallet20k;
it excludes ancillary queries and decoding.

Profiling preceded edits. The copied baseline probe's SHA-256 equals
loader-2's recorded `47791880351152ce1a70b9c64863c2ad1cc6e7b1dd3c07439b0eb63f2cbaca4a`.
`pmp.py` collected 400 baseline samples and 362 optimized samples on repeated
timing-only generic20k requests. The optimized loop finished before its requested
400th sample. Both profiles include book open and final drop, and percentages
overlap. Optimized symbols lack source-line detail; unresolved libc frames
account for 35.8% / 29.3% of self samples, so these are not allocator-only costs.

| Inclusive frame | Baseline | Shared columns |
| --- | ---: | ---: |
| Row construction | 58.5% | 69.9% |
| SQLite stepping | 20.2% | 25.1% |
| Admission | 9.8% | 2.5% |
| Scalar decoding | 10.5% | 25.1% |
| Python-compatible float formatting | 3.5% | 9.1% |

B-tree bulk construction's self samples fall from 8.8% to 0.6%. Row decoding,
SQLite stepping, float formatting and remaining JSON consumers now dominate a
smaller total. The measured SQL floor is not a proof of the loader's minimum.
Raw profiles and disposable books stay under `target/r4-loader-3`.

## Loader-3 verification

The shared-record change passes the 612-test custody/journals suite, followed
by all three focused shared-record/admission tests. The live Python database
oracle passes **44 tests**, including SQLCipher root/pending-write copies and
both >50k worklist paths. All four benchmark books retain exact bytes:
6,793,240 / 9,238,672 / 68,023,010 / 76,513,056 bytes respectively.
Workspace/all-target offline clippy with the loader's `test-harness` and
`-D warnings` passes. The full Rust workspace suite passes **1,504 tests**,
with no failures or ignored tests. Regenerating the **37 frozen SQL fixtures
and four frozen refusals** against Python produces no changes. The twelve
inherited raw generator suites pass all
**24,000 books and 1,030 permutations**. Read-only retained replay compares
**159 requests and 159 permutations**, with zero differences and zero
pre-request refusals.

The loader mutation catalogue grows from 59 to 69 entries. Two existing
configuration-marker recipes are mechanically re-anchored to vector assignment.
The ten new recipes target shared columns, lookup/value pairing, serialization
and configuration admission. Its runner includes custody because the shared
record and admission witnesses now live there. All **69 loader mutations**
build and are killed by their named witnesses; all ten unmutated witness filters
pass first, with no survivors or runner errors. This is still a partial branch
catalogue. The inherited custody catalogue contains 2,292 entries in this
checkout; its profile projection recipe is mechanically re-anchored to the
explicit compatibility conversion. Applicability passes for both catalogues.
The re-anchored `projection8-profile-field` mutation also builds and is killed
by its named witness after an unmutated pass.
The complete custody mutation campaign has not been rerun at this checkpoint.

The required Rust-port inventory update completes without an inventory diff,
and the CI preflight passes **57 tests**. Rust formatting and `git diff --check`
pass. A manual second review covers the shared record, admission order, direct
reader and compatibility conversions.

The loader-2 census/shadow evidence remains inherited: eight shard runs with
1,659 reported exact matches, 37 frozen SQLite fixtures and four frozen exact
refusals. See the historical [loader-2 timing records](r4-native-loader-2-bench.json)
and `0b78605a0` for its full checkpoint. New results do not imply that those
entire shard pairs were rerun.

Verification uses Rust 1.99.1, locked Python 3.11, offline Cargo and
`MATURIN_NO_INSTALL_RUST=1`, without changing `HOME`. Only one test job runs at
a time; scratch and temporary data stay under this worktree's `target`.
After writing verification summaries, the debug/release build trees and mutation
scratch trees are removed. The standalone probe, synthetic books, profiles and
logs remain under `target/r4-loader-3`.
No push, PR, external comment or production authority switch is in scope.

## Loader-4 corrections and coverage

Python's loader filters configuration fields to `chain`, `network`, `samourai`,
`chain_instance_id`, `bitcoin_network` and `chain_network`; within Samourai
metadata it retains only `role`, `group_id` and `section`. It also removes
`_closed_confirmation_observation` from every `_safe_row`. The existing config
projection mirrors the first two rules. Both native safe-row paths now mirror
the last rule, including a physical BLOB column. SQLite TEXT still undergoes
UTF-8 validation before removal. Raw component and revision headers retain the
same-named column. Five frozen cases cover NULL, INTEGER, REAL, TEXT and BLOB
on active transactions and excluded component anchors.

JSON ownership configuration scalars now retain Python's `AttributeError`, and
non-iterable JSON address scalars retain `TypeError`, with their exact messages.
Summary configuration failures follow Python's unordered summary query rather
than the separately ordered derivation wallet query. Inventory output indices
retain integer-conversion families/messages, quote selection, the 200-character
repr precision, nonfinite refusals and digit-limit precedence over an invalid
suffix. Successful finite conversion still uses Python-compatible integer
parsing and truncation.

The storage audit found two further ordinary-value discrepancies. Ownership
inventory falls back to the wallet id for an empty label; receipt inventory
stringifies its stored label, including NULL and empty TEXT. These source-specific
rules now have independent frozen witnesses. Address-list and descriptor source
labels retain their existing rules.

The frozen corpus grows from 37 byte fixtures/four refusals to **59 byte
fixtures/22 exact refusals**. Descriptor additions cover twelve malformed,
scalar, alias, gap-limit and change-synthesis configurations. Source-index
additions cover duplicate inventory, different branch identities, missing and
foreign-profile wallets, active transaction counts, deprecated flags and both
label edge cases. Existing high-water, retired-policy, identity and cache cases
remain in the corpus.

The loader catalogue now has **90 entries**, including these 21 new recipes:

| Contract | Mutation IDs | Named witness |
| --- | --- | --- |
| Safe/raw column removal and discarded TEXT validation | `JL-safe-confirmation-column`, `JL-raw-confirmation-column`, `JL-safe-row-confirmation`, `JL-dropped-text-validation` | `native_loader_filters_only_safe_confirmation_columns` |
| Configuration/address refusal families, messages and summary order | `JL-owned-config-refusal-family`, `JL-owned-config-refusal-message`, `JL-owned-address-refusal-family`, `JL-owned-address-refusal-message`, `JL-owned-summary-refusal-order` | `native_loader_preserves_receipt_error_precedence` |
| Ownership/receipt inventory labels | `JL-owned-empty-inventory-label`, `JL-receipt-raw-inventory-label` | `native_loader_matches_python_database_bytes` |
| Output-index refusals, repr, digit limits and nonfinite conversion | `JL-owned-outpoint-refusal`, `JL-outpoint-repr-limit`, `JL-outpoint-digit-limit`, `JL-outpoint-nonfinite`, `JL-outpoint-nan-family`, `JL-outpoint-infinity-family` | `native_loader_preserves_receipt_error_precedence` and `native_loader_outpoint_integer_preserves_python_refusals` |
| Oversized numeric-prefix refusal precedence | `JL-outpoint-prefix-limit`, `JL-outpoint-prefix-sign`, `JL-outpoint-prefix-unicode-space`, `JL-outpoint-prefix-parse` | `native_loader_outpoint_integer_preserves_python_refusals` |

This maps the added contracts to their recipes; a complete inventory of every
loader branch and its named kill remains open.

## Loader-4 final paired timing

[`r4-native-loader-4-bench.json`](r4-native-loader-4-bench.json) records five
alternating Python/native pairs and five SQL-floor repetitions per book, using
copies of the same seed-1 generic and seed-7 multi-wallet books. All four books
retain exact Python bytes and the loader-3 byte lengths/hashes. The release
probe's hash also matches the storage-audit record. The timing boundary and
best-CPU-sample selection follow the loader-3 protocol above.

| Book | Python CPU / wall | Native load CPU / wall | Admission CPU / wall | Native total CPU / wall | 1-minute load |
| --- | ---: | ---: | ---: | ---: | ---: |
| generic 2,000 | 0.107 / 0.108 | 0.027 / 0.027 | 0.001 / 0.001 | **0.028 / 0.028** | 10.28–11.14 |
| multi-wallet 2,000 | 0.152 / 0.153 | 0.054 / 0.054 | 0.002 / 0.002 | **0.056 / 0.056** | 11.14–11.14 |
| generic 20,000 | 1.159 / 1.169 | 0.281 / 0.285 | 0.009 / 0.009 | **0.290 / 0.293** | 11.21–13.84 |
| multi-wallet 20,000 | 1.400 / 1.416 | 0.415 / 0.419 | 0.024 / 0.024 | **0.439 / 0.444** | 13.20–13.48 |

Unrounded native 20k totals are **0.290061 s CPU / 0.293467 s wall** and
**0.438885 / 0.443591 s**. CPU increases **0.37%** and **2.30%** from the supplied
0.289/0.429 s baselines, below their respective 5% ceilings of 0.30345/0.45045 s.
These are separate shared-host runs; each current Python/native comparison is
paired. The four-query floor is 0.076501 s CPU / 0.077615 s wall at generic20k
and 0.092017 / 0.093013 s at multi-wallet20k.

## Loader-4 verification and storage audit

The custody/journals library and integration suite passes **616 tests**. The
live loader oracle passes **84 tests**, including SQLCipher, pending writes and
both >50k worklist paths. All **59 byte fixtures and 22 exact refusals** regenerate
unchanged under `PYTHONHASHSEED=1` and `2`. Offline all-target clippy for
`kassiber-journals` and `kassiber-custody`, with the loader's `test-harness` feature
and `-D warnings`, passes.

All **90 loader catalogue entries build and are killed** by their named witnesses:
**90 killed, zero survivors, zero runner errors**. All twelve unmutated witness
filters pass first. The [catalogue](../../crates/journals/scripts/loader_mutations.json)
records each mutation's source anchor, rule and witness; the table above maps
the added contracts. This is catalogue coverage, not an exhaustive source-branch
census. The full 2,292-entry custody campaign has **no valid completed result**
for loader-4. An interrupted attempt and a later baseline using stale mutant
artifacts were discarded; neither supplies campaign kills, survivors or errors.
Future batches must start with a fresh build tree. The loader campaign's build
tree has been removed.

[`r4-native-loader-4-storage-audit.json`](r4-native-loader-4-storage-audit.json)
records the remaining storage gaps by fixture, column and requested storage
class. The executable audit uses two synthetic fixtures and SQLite's authorizer
to inventory **295 distinct physical read columns** (292 structural and 229
source fixture/column pairs). Every distinct column has a populated representative
in at least one fixture. Nine storage expressions produce **4,689 attempts**;
**1,107 attempts hit empty tables** and are excluded from the populated counts:

| Populated-row result | Cases |
| --- | ---: |
| Exact v1 value | 2,799 |
| Exact refusal family, code and message | 6 |
| Different refusal or refusal replaced by later admission | 612 |
| Native refusal of a Python-serializable value | 165 |

The **777 mismatches** comprise 350 BLOB, 350 invalid UTF-8, 38 positive infinity,
37 negative infinity and two ordinary TEXT cases. No differing v1 value remains
in these fixtures. These counts do not establish exhaustive column fidelity:
the audit mutates only the first physical row, a predicate or join mutation can
exclude that row, SQLite affinity can coerce the requested class, and only
fixture-reached columns are inventoried. Full local records include `typeof()`,
changed-row counts, reference phase and separate native admission outcomes.

Concrete remaining failures include `transactions.note` BLOB (Python JSON
serialization raises `TypeError: Object of type bytes is not JSON serializable`,
while native loading emits a generic BLOB refusal), invalid UTF-8 in that column
(Python's exact column-bearing `OperationalError` versus the generic native
message), and infinite `transactions.fiat_rate` (Python retains it, native loading
refuses). BLOB wallet configuration can be ignored or normalized by Python while
the native reader refuses it. On authoritative source transactions, TEXT amount
and fee raise Python's `ValueError` during loading; native loading instead reaches
a later generic admission refusal. The audit distinguishes load failures from
v1 serialization failures. These are native parity gaps, not identified Python
reference bugs.

Reproduce the gap inventory with the locally built release probe, keeping
`TMPDIR` inside this worktree's `target` and using the prescribed toolchain:

```sh
TMPDIR="$PWD/target/r4-loader-4/tmp" \
UV_CACHE_DIR="$PWD/target/r4-loader-4/uv-cache" \
MATURIN_NO_INSTALL_RUST=1 CARGO_NET_OFFLINE=true PYTHONHASHSEED=1 \
mise exec rust@1.99.1 uv@latest -- \
  uv run --locked python crates/journals/scripts/audit_loader_storage.py \
  --binary target/r4-loader-4/custody-load \
  --out target/r4-loader-4/storage-audit.json \
  --summary-out docs/reference/r4-native-loader-4-storage-audit.json
```

The audit intentionally exits **1** while any mismatch remains. It reads only
synthetic fixtures, writes no owner book and installs no recording hook.

The required `tests.rust_port.classify_tests update` regenerates the inventory
and corrects its obsolete performance-failure description. The CI preflight
passes **57 tests** after that update. Rust formatting and `git diff --check`
pass. Manual review covers the direct/safe reader, source-specific labels,
summary refusal order, integer conversion and the audit's coverage limits.
Verification and mutation build trees are removed after summarizing; only the
standalone probe, synthetic books and local evidence remain under
`target/r4-loader-4`. No background test job remains at handoff.

## Loader-5 corruption boundary and audit

Acceptance compares the CLI and daemon envelope **code and rendered message**,
not internal exception classes or tracebacks. `cli.main.main` wraps unexpected
exceptions in `internal_error` before calling `core.runtime.emit_error`;
`daemon.run` uses the same code and also redacts the message through
`daemon._error_envelope`. The live oracle exercises both actual product wrappers,
including a message containing synthetic secret material. The native probe
renders its own boundary projections through `crates/envelope`; the audit does
not manufacture native envelopes using Python.

Native UTF-8 refusals now preserve Python's physical result-column order,
including duplicate and discarded columns. Error rendering follows CPython
sqlite3's 198-byte payload limit, C-string NUL termination and per-byte ASCII
replacement, including Unicode column names and truncated long values.
Physical-column scanning runs only on refusals, retaining the original fast
decoder and row allocation on valid books. Optional gap-review
decoding failures retain Python's empty-list fallback.

The loader checks current confirmation quantities after reading the complete
transaction result, before loading the wallet list and ownership index. It uses
the shared graph hash and Python-compatible integer parser, preserves amount/fee
precedence, and skips stale authority versions or graph hashes. The two old
TEXT amount/fee admissions now refuse before snapshot output. Non-TEXT revision
references also fail closed before revision-header lookup, closing eight more
admissions exposed by the expanded audit. This last guard is conservative:
Python's mixed-type sort refusal and all-numeric accepted identities still need
lossless representation and envelope parity; the guard is not acceptance.

The new audit covers all **360 physical columns of 23 loader tables**, including
columns discarded by projections, every physical row of both small synthetic
fixtures, and both declared-affinity and no-affinity schemas. Eleven scalar
expressions produce **22,528 populated cases**, with `typeof()` recorded for
each. Every physical column has a populated representative; authorizer reads
must be a subset of the explicit table/column manifest. Safe/raw physical
confirmation columns are included. BLOB integer and JSON contents supplement
opaque BLOBs. A predicate or join change can still exclude the mutated row;
such equality establishes filtering, not downstream scalar support. Competing
corrupt cells and every optional historical schema remain further work.

| Expanded populated-row result | Cases |
| --- | ---: |
| Exact v1 value | 15,308 |
| Exact CLI/daemon refusal code and rendered message | 1,474 |
| Different refusal | 3,414 |
| Native refusal of a Python-serializable value | 2,332 |
| Native admission of a Python-refused value | 0 |
| Different v1 value | 0 |

The **5,746 unresolved cases** still block acceptance. All 2,048 invalid-UTF-8
cases match, including filtered rows. For the 2,332 native-refused Python values,
Python's downstream projection refuses 1,372 and accepts 960; these outcomes
do not match the native boundary and are not waived. Restricting the expanded
run to the original nine expressions, declared affinities, baseline-read columns
and first physical rows leaves **425 of the original 777 mismatches**: 262
different refusals and 163 native refusals of Python-serializable values.
Actual stored classes across the expanded run are 6,144 BLOB, 1,175 INTEGER,
2,048 NULL, 3,565 REAL and 9,596 TEXT. No physical column lacks a populated case.

Detailed cases append incrementally to `target/r4-loader-5/storage-audit.jsonl`;
the committed [summary](r4-native-loader-5-storage-audit.json) contains the
inventory, actual storage-class totals and remaining failures. Python-accepted
values that native refuses also record the pure Python downstream projection
outcome. A later refusal counts as equivalent only if both boundary projections
match. Snapshot acceptance alone never establishes a Python reference bug.

In particular, a synthetic one-BTC buy with infinite `transactions.fiat_rate`
loads and serializes in Python and enters its transaction-derived rate map as
`Decimal('Infinity')`, but both finalized projection and ledger building refuse
with `validation: transaction nonfinite-rate: fiat_rate must be a finite decimal number`
(also for negative infinity). No unsafe Python tax output was demonstrated;
this is a native downstream-refusal parity gap, not a justified exception.
Other Python-accepted BLOB/nonfinite cases still need individual downstream
resolution. No Python reference-bug waiver is claimed at this checkpoint.

The frozen corpus now contains **61 byte fixtures and 31 refusals**. The
catalogue has **102 applicable loader mutations**; these added/updated contracts
are not a complete loader branch inventory:

| Contract | Mutation IDs | Named witness |
| --- | --- | --- |
| UTF-8 column/message rendering, length, NUL and replacement | `JL-utf8-column`, `JL-utf8-limit`, `JL-utf8-nul`, `JL-utf8-ascii` | `native_loader_utf8_refusals_preserve_python_column_order_and_message` |
| Physical order and auxiliary decoding | `JL-utf8-physical-order`, `JL-utf8-auxiliary-validation` | `native_loader_utf8_refusals_preserve_python_column_order_and_message` |
| Discarded column decoding | `JL-dropped-text-validation` (re-anchored) | `native_loader_filters_only_safe_confirmation_columns` |
| Optional review fallback | `JL-utf8-review-fallback` | `native_loader_utf8_optional_reviews_preserve_python_fallback` |
| Confirmation capture, authority guards and field precedence | `JL-confirmation-capture`, `JL-confirmation-version`, `JL-confirmation-graph`, `JL-confirmation-field-order` | `native_loader_preserves_receipt_error_precedence` |
| Revision reference storage guard | `JL-revision-reference-storage` | `native_loader_refuses_nontext_revision_references_before_output` |

## Loader-5 timing and verification

The [paired timing record](r4-native-loader-5-bench.json) uses copies of the same
seed-1 generic and seed-7 multi-wallet books, five alternating Python/native
pairs per book, and five SQL-floor repetitions. All four retain the exact
loader-4 byte lengths and hashes. The timer still includes native admission and
excludes open/BEGIN and the test-only v1 serializer.

| Book | Python CPU / wall | Native load CPU / wall | Admission CPU / wall | Native total CPU / wall |
| --- | ---: | ---: | ---: | ---: |
| generic 2,000 | 0.111 / 0.112 | 0.028 / 0.028 | 0.001 / 0.001 | **0.029 / 0.029** |
| multi-wallet 2,000 | 0.154 / 0.156 | 0.053 / 0.053 | 0.002 / 0.002 | **0.055 / 0.056** |
| generic 20,000 | 1.160 / 1.170 | 0.281 / 0.283 | 0.009 / 0.009 | **0.290 / 0.292** |
| multi-wallet 20,000 | 1.414 / 1.430 | 0.418 / 0.425 | 0.024 / 0.024 | **0.442 / 0.449** |

Unrounded 20k totals are **0.289819 s CPU / 0.292004258 s wall** and
**0.442404 / 0.449338750 s**. Relative to the supplied 0.290/0.439 s baselines,
CPU changes are **-0.06% / +0.78%**, within 5%; generic retains the 0.30 s target.
Shared-host load during these pairs is recorded in the artifact. Final numbers
use the original row allocation and fast decoder; error-order scanning runs
only when a row refuses.

The final-code custody/journals suite passes **619 Rust tests** and the live
Python loader oracle passes **100 tests**, including actual CLI/daemon wrappers
and refusal assertions at their code/message boundary. All **61 byte fixtures
and 31 refusals** regenerate under `PYTHONHASHSEED=1` and `2`. The final decoder
also passes all **13 focused loader tests** and all-target offline clippy with
`-D warnings`. The required test-inventory update and CI preflight run again
before the checkpoint commit; all **57 preflight tests** pass. Rust formatting,
manual second review and `git diff --check` pass. The full generated/replay/
census/shadow campaigns have not been rerun at this checkpoint; their earlier
counts above remain historical.

All **102 loader catalogue entries build and are killed**: **102 killed, zero
survivors, zero runner errors**. All 15 unmutated witness filters pass first.
The complete run uses a fresh build tree, records each entry incrementally in
`target/r4-loader-5/loader-mutations-final.log`, and removes its scratch and build
trees afterward. It does not replace the missing exhaustive branch inventory
or the full 2,292-entry custody campaign, which has not been run for loader-5.

The standalone probe remains at `target/r4-loader-5/custody-load`; its SHA-256
matches both committed timing and storage-audit records. The verification build
tree and disposable test scratch are removed after summarizing, preserving the
synthetic books, audit JSON/JSONL and verification logs. No background job remains
at handoff.

## Loader-6 boundary continuation (unaccepted)

SQLite infinities now retain their sign through the shared record type as
arbitrary-precision JSON numbers with overflowing exponents. The existing
custody codec restores the original float, and the test-only v1 encoder writes
Python's `Infinity`/`-Infinity` constants. This preserves the value rather than
inventing a rate or an earlier finite-decimal message. A one-BTC buy with an
infinite rate round-trips exactly and retains Python's downstream
`validation: transaction nonfinite-rate: fiat_rate must be a finite decimal number`
refusal when replayed through the reference projection. Native finalized
projection still reports only `AppError` for this case; its exact downstream
code/message parity remains open.

The initial infinity probe exposed 36 load-time admissions in authority-version
and receipt scan-index conversions. Those paths now preserve `OverflowError`
while catching only the `TypeError`/`ValueError` cases Python catches. The same
rule applies to embedded current/historic scan depths and checkpoint horizons.
These refusals are owned by the converting stage, not synthesized from the
finite-decimal validation message.

Numeric revision references keep their original SQLite INTEGER/REAL binding,
numeric ordering, and Python set equality. Comparisons preserve integers above
2^53, signed zero, fractions, i64 endpoints and infinities; equal numeric
references retain their first binding type. Mixed text/numeric populations
remain fail closed under the existing guard pending exact Python sort-refusal
parity. BLOBs still need a lossless raw value representation, including values
Python discards before projection.

The `custody-load` probe's `project` mode uses the directly loaded records,
retains native snapshot admission, runs custody through finalized projection,
and proves no writes or caller-transaction completion before rollback. It
reports the native stage's own refusal and CLI/daemon code/message projections.
Successful projections use the existing test serializer's nonfinite tags and
ASCII text to preserve surrogate code points. The v1 result is a separate
transport gate. Neither a matching snapshot nor a later unrelated refusal can
substitute for a matching projection outcome.

`audit_loader_storage.py` now defaults to this composed comparison, including
Python projections for snapshots that cannot be JSON-serialized. It retains
the same 23-table, 360-column, all-row, two-affinity inventory, records zero
writes per case, and compares successful projection hashes with numeric types
preserved. `--snapshot-only` reproduces the historical transport-only audit.
This is a loader-through-finalized-projection gate, not evidence for the
unrouted journal command, ledger output or persistence.

The frozen corpus contains 66 byte fixtures and 43 load refusals. The loader
catalogue has 123 applicable entries. New witnesses cover infinity transport,
numeric revision ordering/binding, overflow propagation and the separate
projection/transport comparison. This remains a partial branch inventory.

| Added or changed contract | Mutation IDs | Named witness |
| --- | --- | --- |
| Positive/negative infinity and impossible SQL NaN | `JL-direct-nonfinite`, `JL-negative-nonfinite`, `JL-nan-storage` | `native_loader_preserves_nonfinite_real_until_downstream` |
| Integer, REAL and mixed numeric ordering | `JL-revision-integer-order`, `JL-revision-real-order`, `JL-revision-mixed-integer-real`, `JL-revision-mixed-real-integer` | `native_loader_revision_order_preserves_python_numeric_identity` |
| Numeric endpoints, fractions and identity above 2^53 | `JL-revision-upper-range`, `JL-revision-lower-range`, `JL-revision-fraction`, `JL-revision-exact-integer` | `native_loader_revision_order_preserves_python_numeric_identity` |
| Text identity, order and unsupported shapes | `JL-revision-text-order`, `JL-revision-is-text`, `JL-revision-unsupported` | `native_loader_revision_order_preserves_python_numeric_identity` |
| Original SQLite revision binding | `JL-revision-bind` | `native_loader_revision_queries_keep_sqlite_binding_types` |
| Mixed text/numeric guard | `JL-revision-reference-storage` | `native_loader_refuses_mixed_revision_references_before_output` |
| Catch only TypeError/ValueError, propagate infinity overflow | `JL-integer-overflow` | `native_loader_caught_integer_errors_still_propagate_overflow` |
| Authority, receipt branch and receipt MAX overflow | `JL-confirmation-overflow`, `JL-receipt-branch-overflow`, `JL-receipt-index-overflow` | `native_loader_integer_overflow_refusals_match_python` |
| Current/historic scan floor and checkpoint overflow | `JL-current-scan-overflow`, `JL-historic-scan-overflow`, `JL-checkpoint-overflow` | `native_loader_preserves_receipt_error_precedence` |

### Loader-6 composed storage audit

The [current summary](r4-native-loader-6-storage-audit.json) records all
**22,528 cases**, all 360 physical columns of the same 23 tables, every
synthetic row and both affinity modes. Every case proves zero writes. The
acceptance gate now uses the composed projection outcome; the v1 transport
comparison remains diagnostic and cannot override that outcome.

| Loader plus finalized projection | Cases |
| --- | ---: |
| Equal successful projection, preserving numeric types | 6,000 |
| Equal CLI/daemon refusal code and rendered message | 11,850 |
| Different refusal | 3,230 |
| Native refusal of a Python-accepted projection | 1,448 |
| Native admission of a Python-refused projection | 0 |
| Different successful projection | 0 |

**4,678 composed outcomes remain unresolved.** Of these, 4,446 involve BLOBs
(1,482 each for opaque, integer-content and JSON-content bytes). The other
232 comprise 168 component integer-refusal messages, 36 earlier strict
amount/fee admission refusals, 12 component creation-time sort refusals and
16 mixed revision-reference sort refusals. Sixteen of the strict amount/fee
cases have a successful Python projection. None is waived, and none is
resolved by manufacturing a downstream message in the loader.

For comparison with loader-5's historical transport measure, v1 results are
16,540 equal values, 1,526 equal refusals, 3,362 different refusals and
1,100 native refusals of Python-serializable values: **4,462 transport gaps**,
down from 5,746. There are zero native admissions of Python-refused v1 values
and zero different successful v1 values. Transport and projection totals have
different meanings and must not be combined. Under the original audit scope,
351 transport gaps and 402 composed gaps remain across 3,609 cases.

Detailed cases append to `target/r4-loader-6/storage-audit.jsonl`; the complete
result and summary are beside it. The audit exits unsuccessfully for the
remaining composed gaps. Its successful projections do not establish parity
for ledger output, journal persistence or the unrouted product command.

### Loader-6 verification

The [paired timing record](r4-native-loader-6-bench.json) covers disposable
seed-1 generic and seed-7 multi-wallet 20k books, five alternating Python/native
pairs per book and five SQL-floor repetitions. Native timing includes admission
and excludes open/BEGIN and v1 serialization, as before.

| Book | Python CPU / wall | Native load CPU / wall | Admission CPU / wall | Native total CPU / wall |
| --- | ---: | ---: | ---: | ---: |
| generic 20,000 | 1.175 / 1.188 | 0.279 / 0.283 | 0.009 / 0.009 | **0.288 / 0.291** |
| multi-wallet 20,000 | 1.454 / 1.479 | 0.413 / 0.416 | 0.024 / 0.024 | **0.437 / 0.440** |

Unrounded native totals are **0.288152 s CPU / 0.291321532 s wall** and
**0.436581 / 0.440290602 s**. Relative to the supplied 0.290/0.442 s CPU
baselines, changes are **-0.64% / -1.23%**, within 5%; generic retains the
0.30 s target. The one-minute host load is 74.84–91.49 across these pairs.
Both books pass the complete Python/native v1 byte gate. Their generator CSV
hashes and v1 byte lengths match loader-5; their snapshot hashes differ, so
this run does not claim identical persisted facts to the historical books.
A separate run of the rebuilt parent probe on these same copies produces
identical v1 bytes and CPU totals of 0.277136 / 0.429969 s. Current CPU is
**+3.97% / +1.54%** against that run, also within 5%. All parent samples and
its binary hash are retained under `rebuilt_parent` in the timing record.

All **123 loader catalogue entries build and are killed**, with zero survivors
and zero runner errors. All 20 unmutated witness filters pass first. The run
records each entry incrementally in `target/r4-loader-6/loader-mutations.log`;
`loader-mutations-summary.json` beside it retains each named witness and kill.
The dedicated build tree and disposable mutation checkout are removed after
summarizing. This is the partial loader catalogue, not the full 2,292-entry
custody campaign or an exhaustive loader branch map.

The final Rust source passes **624 custody/journals tests**. The live Python
loader oracle passes **121 tests**, and the final composed-audit gate assertions
pass their focused recheck. All **66 byte fixtures and 43 load refusals**
regenerate under `PYTHONHASHSEED=1` and `2`. All-target offline clippy with
`test-harness` and `-D warnings`, Rust formatting, manual second review and
`git diff --check` pass. The required inventory update and CI preflight run
before the checkpoint commit; all **57 preflight tests** pass. Logs remain
under `target/r4-loader-6/`.

The standalone probe is retained at `target/r4-loader-6/custody-load`; its
SHA-256 matches the current timing and storage-audit records. Verification and
mutation build trees and disposable test scratch are removed after summarizing;
logs, synthetic benchmark books and standalone probes remain. No background job
remains at handoff. The generated/replay/census/shadow counts in earlier
sections remain historical: those full campaigns have not run for loader-6.

## Remaining acceptance work

1. Finish lossless legacy `SqlValue` storage and product-boundary refusal
   code/message parity, including BLOB, nonfinite downstream errors, mixed
   revision references, ignored/overwritten fields and competing invalid rows.
   Match Python-accepted values' downstream outcomes; document a Python reference bug
   with a repro only when unsafe output is demonstrated. The expanded audit
   remains an executable inventory of open gaps, not an acceptance result.
2. Finish an explicit branch-to-named-mutation map; 123 applicable entries are
   still a partial catalogue. Run the full 2,292-entry custody campaign after
   behavior is settled, in resumable shards with incremental per-entry logs and
   fresh build trees, deleting each shard's build tree after summarizing.
3. Complete the remaining parity/review gates after the lossless boundary and
   exhaustive mutations are implemented: 24,000 generated books plus 1,030
   permutations, the 159 retained replays under
   `/home/tg/.cache/kassiber-verify/custody-recordings/`, and the census/shadow
   gates. Those inherited counts remain historical evidence until rerun.
   Finalize acceptance and update plan 21/TODO only after these gates pass.

## R4-13 integration contract

R4-13 must open through `crates/store`, hold `BEGIN IMMEDIATE` and its build
savepoint, run R4-10 repairs/pricing, then call the loader on that same connection
with the exact ordered profile and injected `as_of`. It must consume the eventual
lossless typed snapshot directly, roll back refusals/panics without refreshing
freshness markers, and commit only after successful persistence. CLI/daemon/AI
routes must not call this test-only prototype or adopt its serializer as a
production boundary. Accounting admission and watch-only/no-egress rules remain.
