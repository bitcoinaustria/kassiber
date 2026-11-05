# R4-9 native custody loader checkpoint

> Source acceptance is inherited from t3code/r4-9-loader-determinism@aaf6216e3.
> Fresh R9 verification passes all 274 loader/BLOB cases, the 24,000-book
> custody/render campaign with 1,030 permutations, 159 archived requests and
> their permutations, and focused census/shadow. The
> [R9 integration audit](rust-backend-r9-integration.md#fresh-r9-verification)
> tracks the remaining combined gates; historical timings below retain their
> source scope.

R4-9 is **fully accepted, with no pending exceptions**. The integrated
continuation combines loader-9's exhaustive branch map and stage campaigns,
the BLOB continuation's lossless consumers, and the deterministic revision
refusal below. The [current acceptance record](r4-native-loader-determinism-verification.json)
owns the fresh integrated gates. Earlier sections retain their historical
measurements and refusal rules; the [deterministic continuation](#deterministic-revision-reference-acceptance)
supersedes their forty message exceptions. Production journal authority remains
with Python; merged full-process integration belongs to R4-13.

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
remain distinct. Mixed revision references now refuse with one typed
`validation` error in the port-branch Python reference and native loader before
revision sorting/queries. INTEGER and REAL remain one sortable numeric family.
BLOB parity and all forty historical message exceptions are closed.

Admission memoizes only successfully checked, identical configuration strings
within one validation call. Every different string and every object still goes
through the same configuration allowlist. Wallets, transactions, anchor wallets,
channels, gap rows and anchor objects retain both configuration checks. The
JSON adapter requires exact signed-64 msat principal/fee quantities; loader-7
retains database scalars until their consuming stage converts or refuses them.
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

## Loader-7 non-BLOB continuation (unaccepted)

Database admission now shares schema/configuration validation with the JSON
adapter but retains the original SQLite amount/fee scalars for their consuming
stages. The JSON adapter still requires exact signed-64 msat quantities. Native
loading does not convert a legacy REAL or TEXT quantity into an accounting fact;
the existing Python-equivalent consumer converts or refuses it. Component and
evidence integer conversions preserve invalid-literal messages and uncaught
infinity overflow. Component creation-time sorting compares the original tuple
operands and preserves Python's TypeError instead of sorting their strings.

Finalized base-row pricing carries a structured code/message refusal from its
own finite-decimal validator through composition and the native probe. Both
infinity signs now produce Python's exact
`validation: transaction nonfinite-rate: fiat_rate must be a finite decimal number`
at finalized projection. No message is manufactured by the loader. Existing
primitive and lossless example callers retain their family-only diagnostic API.

The [non-BLOB composed audit](r4-native-loader-7-storage-audit.json) covers the
same 23 tables, 360 columns, every synthetic row and both affinity modes, with
BLOB expressions explicitly excluded by `--non-blob`:

| Loader plus finalized projection | Cases |
| --- | ---: |
| Equal successful projection, preserving numeric types | 5,308 |
| Equal CLI/daemon refusal code and rendered message | 11,060 |
| Different refusal | 16 |
| Native admission of a Python-refused projection | 0 |
| Different successful projection | 0 |
| Cases proving zero writes | 16,384 |

All **216 non-BLOB gaps outside mixed revision sorting are resolved**: 168
component integer messages, 36 earlier amount/fee admissions and 12 component
creation-time sort refusals. The 16 remaining cases involve INTEGER/REAL
`supersedes_component_id` or `superseded_by_component_id` among TEXT references
in the no-affinity structural fixture. The existing guard still refuses before
output. It has not been replaced by a guessed message.

The [four-seed reference probe](r4-native-loader-7-mixed-revision-seeds.json)
records a concrete complication: the same stored `1.5` reference reports
`'<' not supported between instances of 'str' and 'float'` under
`PYTHONHASHSEED=1`, but reverses those types under seed 2. Negative infinity also
changes direction. Every run refuses. This is unresolved message fidelity,
not an unsafe Python output or a reference-bug waiver.

The 6,144 BLOB attempts, including **4,446 previously unresolved outcomes**, are
excluded from this run at the owner's direction. Their loader-6 inventory is
retained in [the prior summary](r4-native-loader-6-storage-audit.json); all 4,446
remain **pending an owner decision**. The generic BLOB refusal and fail-closed
boundary remain in place. Neither the smaller audit nor transport equality
establishes BLOB acceptance.

Detailed non-BLOB cases append to `target/r4-loader-7/storage-audit.jsonl`;
the full JSON, reference seed probe and logs remain beside it. Reproduce with
`audit_loader_storage.py --non-blob` and the test-only probe, with `TMPDIR`
inside this worktree's `target`. The audit intentionally exits 1 for the 16
remaining mixed-reference refusals. R4-9 is **not accepted**, including under
the proposed acceptance-except-BLOB wording.

### Loader-7 timing and verification

The [paired benchmark](r4-native-loader-7-bench.json) records five alternating
Python/native pairs for each disposable 20k book. Both books pass the complete
v1 byte comparison before timing. Native total includes admission and excludes
book open, BEGIN and serialization, using the same method as loader-6. The best
complete CPU sample supplies its paired wall measurement.

| Book | Native CPU / wall | Supplied loader-6 CPU | CPU change |
| --- | ---: | ---: | ---: |
| generic 20,000 | 0.279943 / 0.282135 s | 0.288152 s | -2.85% |
| multi-wallet 20,000 | 0.424456 / 0.431536 s | 0.436581 s | -2.78% |

The final audit and benchmark both record probe SHA-256
`6c08c4ef3f09af090bd691990d92ee0c591502e6a8c159fe224414b76d74374b`,
matching the retained standalone probe. All **628 custody/journals Rust tests**
and **205 live Python loader tests** pass. The latter include 84 parameterized
non-BLOB scalar comparisons and native downstream infinite-rate refusals.
Release/all-target offline clippy for both crates with `test-harness` and
`-D warnings` passes. Full generator, retained-replay and census/shadow gates
remain historical until rerun.

Seven additional loader recipes bring its partial catalogue to 130 entries.
All **130 loader mutations** build and are killed by their named witnesses,
after all 24 unmutated witness filters pass, with no survivors or runner errors.
Nine inherited custody recipes are reanchored to the changed conversion,
sorting and structured-error paths; the custody catalogue remains 2,292 entries.
All nine reanchored mutations build and are killed after their nine unmutated
witness filters pass, with no survivors or runner errors. Both catalogues pass
applicability checks. These nine kills do not replace the full custody campaign.
The following additions extend the map without establishing exhaustive branch
coverage:

| Added contract | Mutation IDs | Named witness |
| --- | --- | --- |
| Database quantities reach their consumers through both native entry points | `JL-loaded-admission`, `JL-projection-loaded-admission` | `native_loader_legacy_scalars_keep_downstream_admission` |
| Component infinity conversion preserves uncaught overflow | `JL-component-integer-infinity` | `component_integer_refusal_messages_match_python` |
| Component invalid integer text keeps the rendered literal | `JL-component-integer-message` | `component_integer_refusal_messages_match_python` |
| Timestamp sort refusal keeps comparison direction and operand types | `JL-component-sort-message` | `component_header_sort_retains_python_type_errors` |
| Downstream pricing preserves the validation code and field context | `JL-price-code`, `JL-price-message` | `finalized_price_errors_keep_field_code_and_message` |

The required inventory update adds the scalar regression's oracle entry and
refreshes the disposition totals. CI preflight passes all **57 tests**. Rust
formatting and `git diff --check` pass. A manual second pass reviews shared
snapshot admission, integer catch families, sort operands, structured-error
propagation, the mutation anchors and the audit's separate projection gate.

Verification uses Rust 1.99.1, locked Python 3.11, offline Cargo and
`MATURIN_NO_INSTALL_RUST=1`, without changing `HOME`. Scratch stays under this
worktree's `target`, with one test job at a time. Each mutation shard's fresh
build tree is deleted after its incremental log and summary are written. The
verification build tree and copied uv cache are also deleted after summarizing;
the standalone probe, synthetic books, scripts, full audit cases and logs remain
under `target/r4-loader-7`. No background job remains at handoff. This is a local
checkpoint with no push, PR, external comment or production routing change.

## Loader-8 seed-variant audit rule (unaccepted)

The owner authorizes matching these mixed numeric/TEXT revision-reference
refusals when the native code equals Python's and the native message is one
Python produces across hash seeds. Native loading remains deterministic: it
always renders the numeric operand first, using `int` for SQLite INTEGER and
`float` for SQLite REAL, including both infinity signs. The mixed population
still refuses before revision queries or snapshot output, without writes or
ending the caller's transaction. No Python production implementation changes.

The audit measures each eligible synthetic case in fresh CPython 3.11 processes
under `PYTHONHASHSEED=1,2,3,4`. It retains each complete CLI/daemon code/message
pair by seed and the deduplicated variant set. Only numeric values in the two
revision-reference columns, producing a load-time sort TypeError, qualify.
Successful projections, other columns, other exception families, unobserved
messages and different codes retain the strict comparison. Matching never
combines the CLI half of one run with the daemon half of another.

The [loader-8 non-BLOB audit](r4-native-loader-8-storage-audit.json) records:

| Loader plus finalized projection | Cases |
| --- | ---: |
| Equal successful projection, preserving numeric types | 5,308 |
| Matching CLI/daemon refusal code and rendered message | 11,076 |
| Different refusal or successful projection | 0 |
| Native admission of a Python-refused projection | 0 |
| Cases proving zero writes | 16,384 |
| Matching mixed revision-reference cases, pending Python determinism fix | 16 |

The same 23 tables, 360 columns, physical rows and both affinity modes are
covered. BLOB expressions remain excluded with `--non-blob`; the 4,446 pending
BLOB gaps and the loader-6 inventory are unchanged. Full synthetic cases append
to `target/r4-loader-8/storage-audit.jsonl`; the complete JSON and run log are
beside it. The sixteen pending cases, variant sets and per-seed pairs are
included in the committed summary with their pending status.

The updated native regression repeats every refusal, tests both revision columns
with INTEGER, REAL and both infinity signs, and checks transaction ownership and
zero writes. The audit regression rejects unobserved codes/messages and proves
that a native successful projection cannot use the variant exception.

Six new recipes extend the partial loader catalogue from 130 to 136:

| Contract | Mutation IDs | Named witness |
| --- | --- | --- |
| Mixed-reference refusal family and deterministic message | `JL-revision-message-family`, `JL-revision-message-text` | `native_loader_refuses_mixed_revision_references_before_output` |
| INTEGER/REAL Python type names | `JL-revision-message-int`, `JL-revision-message-real` | `native_loader_refuses_mixed_revision_references_before_output` |
| Measured variant equality, including negative code/message checks | `JL-audit-seed-variants`, `JL-audit-seed-code` | `test_loader_seed_variants_require_observed_complete_refusal_pairs` |

### Loader-8 timing and verification

The [paired benchmark](r4-native-loader-8-bench.json) uses disposable copies of
loader-7's seed-verified 20k books. Both pass the complete v1 byte gate before
five alternating Python/native pairs. Native total includes admission and
excludes book open, BEGIN and serialization; wall is paired with the best
complete CPU sample, using the same method as loader-7.

| Book | Native CPU / wall | Loader-7 CPU | CPU change |
| --- | ---: | ---: | ---: |
| generic 20,000 | 0.288331 / 0.291505 s | 0.279943 s | +3.00% |
| multi-wallet 20,000 | 0.428645 / 0.432649 s | 0.424456 s | +0.99% |

Both stay inside the owner's 5% CPU budget. The benchmark and audit record the
same standalone probe SHA-256
`c8d26ddea18093b66e4164559692d96a8a804793da399e9eeabe6c5ef76c7489`.

All seven selected mutations (the six new recipes plus the existing mixed
population guard) build and are killed, with zero survivors and runner errors.
Both named witness filters pass before mutation. This is a fresh seven-entry
shard, not a fresh run of all 136 loader entries or the full custody catalogue.
Its incremental log and machine-readable summary remain under
`target/r4-loader-8/mutations-loader/`; the shard build/scratch trees are deleted.
The local `run-mutants.py` wrapper there supports resumable shards bound to
catalogue/source fingerprints and checks the kill list against every selected shard manifest.

All **628 custody/journals Rust tests** and **206 live Python loader tests** pass.
A separate final-code seed probe repeats all sixteen cases under four Python
hash seeds (64 fresh reference processes) and repeats each native request twice,
proving stable observed refusals and zero writes. Its synthetic cases and log
remain under `target/r4-loader-8/mixed-revision-seeds.json` and `logs/`.
Offline release/all-target clippy for both crates with `test-harness` and
`-D warnings`, Rust formatting and `git diff --check` pass.

Both catalogues pass applicability: **136 loader entries** and **2,292 custody
entries**. The required test inventory update adds the seed-rule oracle and
refreshes the disposition totals; CI preflight passes all **57 tests**. A manual
second pass reviews the deterministic mixed-reference guard, measured variant
scope, code/message pairing, success/refusal separation, mutation witnesses,
zero-write checks and the unchanged BLOB boundary.

Verification uses Rust 1.99.1, locked Python 3.11, offline Cargo and
`MATURIN_NO_INSTALL_RUST=1`, without changing `HOME`. Scratch remains under
this worktree's `target`, with one test job at a time. Verification build trees
and the copied uv cache are deleted after summarizing; standalone probe,
synthetic books, incremental logs, seed records and scripts remain under
`target/r4-loader-8`. No background job remains at handoff. This is a local
checkpoint with no push, PR, external comment or production routing change.

This extends the named contracts but does not finish the exhaustive branch map.
No full custody mutation, generator, replay or census/shadow rerun is claimed
here. The inherited complete-campaign evidence remains historical.

## Loader-9 acceptance

The original loader-9 acceptance on 2026-11-01 excluded BLOB corruption and
sixteen seed-dependent messages. The BLOB and deterministic continuations below
close both exclusions: **R4-9 is now fully accepted, with no pending exceptions**.
The [verification summary](r4-native-loader-9-verification.json) records the
catalogue hashes, campaign fingerprints, probe hash, gate counts and log paths.
Production Rust and Python code are unchanged in loader-9: changes add test
witnesses, fixtures, mutation recipes and acceptance documentation. The
transitional representation and production-authority boundaries above remain.

### Exhaustive map and mutation campaigns

The [branch map](r4-native-loader-branch-map.md) covers the loader and all five
modules, shared record/admission seams and downstream refusal contracts. It
names each recipe and its killer, groups equivalent alternatives, and explains
delegated, unreachable and representation-only defenses. The loader catalogue
grows from 136 to **308 entries** without removing an entry. New witnesses cover
scalar preservation/refusals, safe configuration, ownership scope and ordering,
descriptor horizons, conservative legacy inference, receipt defaults, SQLite
parameters and cross-asset worklist ranking.

| Final catalogue | Killed | Survivors | Runner errors |
| --- | ---: | ---: | ---: |
| Loader | 308 / 308 | 0 | 0 |
| Custody | 2,292 / 2,292 | 0 | 0 |

All 33 loader witness filters pass unmutated before the final 308-entry run.
Its final shard is `target/r4-loader-9/mutations-loader/0000-0308/`, with
1,484.774 seconds wall time. The earlier exploratory shard is not the final
campaign evidence.

The custody campaign uses nine resumable shards of at most 256 entries, with
incremental, fsynced `results.jsonl`, manifests, run logs and summaries under
`target/r4-loader-9/mutations-custody/`. At the three-hour checkpoint, 1,674
entries are killed, with no survivors/errors and 618 remaining. The resumed
original campaign completes all 2,292 entries: 2,289 kills, three survivors and
zero runner errors. Original segment times are 10,800.695 and 1,676.921 seconds;
the checkpoint summary and complete original result are retained separately.
Every named witness passes unmutated in its shard, covering 408 distinct filters.

Three recipes are re-anchored to their stated contracts, preserving the same
IDs and named killers:

| Recipe | Re-anchor and named killer |
| --- | --- |
| `projection-hash-type-family` | Removing the preliminary conversion no longer changes the malformed-container refusal: the shared component integer converter also returns `TypeError`. The revised recipe explicitly substitutes `ValueError`, killed by `projection_fee_row_vectors` on the existing malformed amount/fee cases. |
| `R7-price-exponent` | The base-row witness now calls `Money::finite_field`, so the recipe targets that method's exponent bound rather than `require_finite`. Existing long-exponent cases in `projection_base_row_vectors` kill it. |
| `R7-price-floor` | The recipe targets `finite_field`'s nonzero precision floor, reached by the existing tiny-price cases in `projection_base_row_vectors`. |

Each revised recipe builds and is killed in a fresh single-entry shard after
its unmutated witness passes. The three resolution shards take 66.584 seconds
combined. Production and witness code are unchanged between the original and
resolution runs, and all other 2,289 recipes are unchanged. The aggregate
summary proves that the union of unchanged original kills and fresh revised
kills equals the final 2,292-entry catalogue; it does not relabel the original
survivors as kills. Both original and resolution fingerprints are retained.
Every shard's build and scratch trees are deleted after summarizing.

### Final parity and verification gates

| Gate | Fresh result |
| --- | --- |
| Twelve generated suites | 24,000 books and 1,030 permutations; 24,852 complete seven-stage matches and 178 matching finalized refusals |
| Per-scenario controls | Exact visit counts, quantity activity/explicit inactive controls and frozen interpretation counts pass |
| Retained replay | 159 requests and 159 permutations; zero differences and pre-request refusals |
| Focused census | 59 tests; 53 builds compare all seven stages with zero skips; 53 exact native-loader v1 byte matches |
| Focused shadow | 59 tests; 53 builds compare all seven stages with zero skips; 53 exact native-loader v1 byte matches |
| Fresh recordings | Each focused lane replays 53 requests and 53 permutations; zero differences and pre-request refusals |
| Custody/journals Rust suite | 636 tests pass; zero failures or ignored tests |
| Loader oracle | 76 byte vectors and 43 refusals match Python; original 66 byte fixtures preserved; 216 live Python tests and 27 native-loader Rust tests pass |
| Static and inventory gates | Offline release/all-target clippy with `test-harness` and `-D warnings`, workspace formatting, both catalogue applicability checks, Rust-port inventory update and 57-test CI preflight pass |

The 178 generated refusals are the existing fanout nonfinite-price validation
cases: **170 originals plus eight permutations**. Python and Rust agree on the
refusal code/stage, and earlier outputs still compare without finalization.
The focused modules are `test_custody_observation_consistency.py` and
`test_source_overlap.py`. They use the explicit standalone loader probe and
disposable recording roots. The retained corpus at
`/home/tg/.cache/kassiber-verify/custody-recordings/` is read-only.

At this historical loader-9 checkpoint, the loader-8 audit and paired timing
were inherited, not rerun or changed. The later continuations below supersede
the recorded storage gaps.
All **16,384 non-BLOB cases** retain their recorded matches, zero writes and
zero unsafe admissions. The **4,446 BLOB gaps** remain pending an owner decision;
the **16 seed-dependent messages** remain pending the Python determinism fix,
under the existing measured-variant rule. No exception is expanded or resolved.
Native 20k CPU including admission remains **0.288331 seconds generic** and
**0.428645 seconds multi-wallet**, both within the owner's 5% loader-7 budget.

Verification uses Rust 1.99.1, locked Python 3.11, offline Cargo and
`MATURIN_NO_INSTALL_RUST=1`, with unchanged `HOME`, disposable worktree-local
data roots and one test job at a time. A manual second pass checks the complete
map, named killers, three re-anchors, aggregate provenance, fixture preservation
and pending exceptions. Verification build trees and the copied uv cache are
deleted after summarizing; scripts, probe, resumable logs and fresh recordings
remain under `target/r4-loader-9`. No background job remains at handoff. This is
a local acceptance commit, with no push, PR, external comment or routing change.

## BLOB storage-class continuation (historical)

> inherited from t3code/r5-transfers-17@59e1476a4; combined R10 results are in the [R10 audit](rust-backend-r10-integration.md).
## BLOB storage-class continuation (original branch evidence)

At the BLOB checkpoint, SQLite BLOBs entered both native readers as `kassiber_pyjson::Value::Bytes`.
The transitional JSON-shaped custody records carry a private, lossless hex
adapter that restores typed bytes at consumers. Literal mappings with private
tag keys are escaped in both the JSON request and native typed-map paths, so
they cannot acquire byte identity or leak adapter tags through serialization.

Consumers apply Python's own policy: normalized configuration ignores bytes;
ownership and stored graph JSON consumers detect UTF-8/16/32 encodings;
integer consumers accept ASCII byte integers and retain bytes repr in invalid
literal errors. Evidence canonicalization separately uses UTF-8 replacement.
No global decoding, stringification or null substitution turns bytes into
accounting facts. JSON without `default=str` refuses bytes; with it, bytes use
Python's repr and quote/escape rules.

For example, a BLOB `transactions.note` survives loading and fails v1 JSON
encoding with `TypeError: Object of type bytes is not JSON serializable`.
Both CLI and daemon render code `internal_error` and the exact message
`Object of type bytes is not JSON serializable`. Native serialization refuses
at that boundary, without output or writes; composed projection results and
refusals are measured independently, so a later unrelated refusal cannot hide
an earlier mismatch.

The [full composed audit](r4-native-loader-blob-storage-audit.json) and
[non-BLOB rerun](r4-native-loader-blob-non-blob-audit.json) cover all 360 physical
columns of the same 23-table manifest, every fixture row, declared affinities
and no-affinity copies. Every detailed case records actual SQLite `typeof()`.
The full run has 7,448 equal successful projections and 15,080 equal refusals.
Of its 6,144 actual BLOB cases, 2,140 produce equal projections and 4,004
produce equal refusals.
Both composed and v1 outcomes match for every BLOB attempt. Both audits have
**zero unresolved cases, zero native admissions of Python-refused projections
and zero writes**. Predicate/join filtering remains distinguished from actual
downstream scalar consumption; combinations of corrupted cells are not exhausted.

Each of the 24 BLOB/TEXT revision cases measures the complete CLI/daemon
code/message pair in fresh CPython 3.11 processes under hash seeds 1, 2, 3 and 4.
Seed 1 renders `str` then `bytes`; seeds 2–4 render `bytes` then `str`. The audit
stores each per-seed pair and the complete measured variant set, without mixing
halves or broadening successful outcomes. Native always renders `str` then
`bytes`, and refuses before revision queries or output. These 24 and the prior
16 cases remained pending at that checkpoint; the deterministic continuation
below closes all forty. The planned port-branch
Python fix will refuse mixed-type revision references with one typed message
before sorting; the adjacent mixed-type guards in `loader/direct.rs` can switch
to that single refusal when it lands. Python production code is unchanged.
The standalone repro and full case logs remain under `target/r4-loader-blob`.

The [focused mutation summary](r4-native-loader-blob-mutations.json) records
**50 kills, zero survivors and zero runner errors**: 47 new BLOB
recipes in `loader_blob_mutations.json` and three reanchored inherited recipes.
All 16 distinct named witnesses pass unmutated.
Both loader catalogues validate (183 entries combined), but remain a partial
map. All 2,292 inherited custody entries also pass applicability.
The summary names each kill, records source/catalogue hashes and the offline
debug build configuration. Mutation build and disposable checkout trees are
deleted after summarizing; this does not claim the full custody campaign.

Fresh verification passes **671 Rust tests**, workspace/all-target release
clippy with `test-harness` and `-D warnings`, and **249 Python loader cases**.
The structured large-worklist case hits the 120-second subprocess wall limit
in the full run (248 pass), then passes its isolated rerun in 73.57 seconds
with complete byte and zero-write checks.
The finalized raw-stage driver compares **24,000 generated books plus 1,030
permutations** across all twelve suites. All **159 retained recordings plus
159 permutations** at `/home/tg/.cache/kassiber-verify/custody-recordings/` compare
with zero differences against the rebuilt native binding.

The [paired CPU evidence](r4-native-loader-blob-bench.json) preserves five
alternating Python/native pairs per book and both retained native probes.
The regression check interleaves fifteen loader-8/current native pairs per
book, reversing order within alternate pairs to reduce host-load drift. Both
probes use the same CPU affinity, selected from a one-second sibling activity
sample. Process user+sys includes native admission and excludes book open and
v1 serialization. Using the best complete CPU sample for each probe, current
20k CPU is **0.297664 s generic / 0.567193 s
multi-wallet**, versus **0.301050 / 0.585340 s** for loader-8 on
this same busy host. Both stay within the 5% regression allowance against the
retained baseline; the artifact also retains the historical 0.280 / 0.424 s
figures and host load. Median paired current/baseline ratios are 1.0091 generic
and 0.9293 multi-wallet. The original absolute 5% limits of 0.294 / 0.4452 s
are not both reproduced on this contended host; the controlled comparison
establishes regression against loader-8, not those historical absolute limits.
TEXT JSON parsing retains its direct allocation path.

The required inventory update and all 57 CI preflight tests pass before the
local commit. Rust formatting and `git diff --check` also pass.
A manual second pass checks byte identity, literal-map collisions, consumer
conversion and refusal stages, configuration privacy, exact CLI/daemon
envelopes, deterministic revision guards, zero writes and mutation anchors.
Verification uses locked Python 3.11, Rust 1.99.1, offline Cargo, eight build
jobs and `MATURIN_NO_INSTALL_RUST=1`, without changing `HOME` or dependencies.
Scratch stays under this worktree's `target/r4-loader-blob`. Build trees and
the copied uv cache are deleted after summarizing, while the probe, synthetic
books, scripts and logs remain. No background job remains at handoff. Local
only: no push, PR, external comment or production routing change.

## Deterministic revision-reference acceptance

On 2026-11-01, **R4-9 is fully accepted, with no pending exceptions**.
The port-branch Python reference `load_custody_snapshot` checks reference
families immediately before sorting revision IDs. Mixed numeric/TEXT,
BLOB/TEXT and numeric/BLOB populations raise `AppError` with code `validation`
and the exact message `Custody revision references contain incompatible types`.
The native loader emits the same typed refusal before revision queries or
output. Both retain the existing earlier-error precedence and caller rollback
ownership. INTEGER and REAL remain mutually sortable, including infinities and
integer identity above 2^53; homogeneous TEXT/BLOB references retain their own
ordering and SQLite bindings. Shipped Python was audited separately and is
unaffected by this port-branch reference correction.

The audit no longer accepts any message variant set. It compares the complete
CLI/daemon code/message pairs exactly, independently for v1 transport and
finalized projection. The [composed audit](r4-native-loader-determinism-storage-audit.json)
and [non-BLOB rerun](r4-native-loader-determinism-non-blob-audit.json) retain the
same 23-table, 360-column, all-row, two-affinity manifest and actual `typeof()`.
The forty closed cases have exactly the same identities as the historical
sixteen numeric/TEXT and twenty-four BLOB/TEXT exceptions. Each case is repeated
in fresh CPython 3.11 processes under **eight hash seeds, 0–7**, requiring the
same family, validation code and message in every process. This includes both
infinity signs and all three BLOB payload sets. The full audit contains 320
fresh seed-process checks; the separate non-BLOB rerun adds 128. Native
family/code/message and default AppError attributes also match all forty cases.

| Fresh gate | Result |
| --- | --- |
| Composed storage audit | 22,528 exact matches: 7,448 successful projections and 15,080 refusals; all 6,144 actual BLOB cases match |
| Non-BLOB audit | 16,384 exact matches: 5,308 successful projections and 11,076 refusals |
| Audit safety and seed closure | Zero writes, unsafe admissions, unresolved outcomes or pending exceptions; all forty historical case identities verified |
| Twelve generated suites | 24,000 books and 1,030 permutations; 24,852 complete seven-stage matches and 178 matching finalized refusals |
| Per-scenario controls | Exact visit counts, quantity activity/inactive controls and frozen interpretation counts pass |
| Retained replay | 159 requests and 159 permutations; zero differences or pre-request refusals; source corpus read-only |
| Focused census and shadow | Each passes 59 tests, 53 exact native-loader v1 matches and 53 comparisons of all seven stages with zero skips |
| Fresh focused recordings | Each lane replays 53 requests and 53 permutations with zero differences or pre-request refusals |
| Live Python loader oracle | 274 tests pass, including the eight-seed refusals and valid mixed INTEGER/REAL control |
| Rust journals/custody/pyjson/pytext/envelope suites | 679 tests pass; zero failures or ignored tests |
| Frozen loader vectors | All 76 byte vectors and 43 refusals regenerate without changes |
| Static and inventory gates | Offline release/all-target clippy with `test-harness` and `-D warnings`, workspace formatting, catalogue applicability, inventory update and 57-test CI preflight pass |

The [fresh loader mutation campaign](r4-native-loader-determinism-mutations.json)
kills **all 357 entries**, with zero survivors or runner errors; all
**46 named witness filters** pass unmutated first. The catalogue retains
all 308 loader-9 and 47 BLOB entries and adds `JL-reference-mixed-type-guard`
and `JL-revision-validation-code`. Removing the new Python guard is killed by
the fresh-process seed witness. Existing mixed-reference recipes are re-anchored
to the typed refusal, and the former variant-rule IDs now enforce exact boundary
matching and BLOB seed coverage. The merge also narrows `JL-integer-string` to
its string arm because BLOB conversion introduced a second parser call.
The [branch map](r4-native-loader-branch-map.md) includes the combined contracts.
The inherited complete **2,292-entry custody campaign** remains the loader-9
record; this continuation does not claim a fresh custody mutation campaign.

The [machine-readable acceptance record](r4-native-loader-determinism-verification.json)
contains source/catalogue/harness fingerprints, the native probe hash, each gate's
log/hash and campaign provenance. The earlier focused-run assertion compared
native-only metadata with the Python refusal projection; it was corrected to
compare family/code/message, and the complete 274-test oracle passes afterward.
Preflight found the static error-code index needed the new validation site and
shifted source locations; the reviewed snapshot is refreshed and all 57 tests pass.
Timing is inherited from the BLOB same-host regression record, which stays
within 5% of its retained loader-8 baseline; no fresh benchmark is claimed.

Verification uses Rust 1.99.1, locked Python 3.11.17 and uv 0.13.6, offline
Cargo, `CARGO_BUILD_JOBS=8` and `MATURIN_NO_INSTALL_RUST=1`, without changing
`HOME` or dependencies. One test job runs at a time. All scratch is under
`target/r4-loader-determinism`; mutation scratch/build trees, verification
build trees and the copied uv cache are removed after summarizing. The native
probe, synthetic audit records, scripts, source-bound incremental mutation
results and logs remain. A manual second review covers guard precedence,
scalar identity, exact boundary pairing, all forty case identities, named kills,
zero writes and authority. No background job remains at handoff. This is local
only: no push, PR, external comment or production authority switch.

> inherited from t3code/r5-transfers-17@59e1476a4; combined R10 results are in the [R10 audit](rust-backend-r10-integration.md).
## Integrated continuation

Transfers-14 integrates loader-9 and the BLOB continuation, including loader-3
through loader-8 prerequisites missing from its transfer base. The loader-9
branch map, all 308 original loader recipes and the 47 original BLOB recipes
are retained. Two review-assembler admission/refusal guards bring the loader
catalogue to 310. A shared-snapshot cache invalidation guard and two
finalized-output identity guards bring the BLOB catalogue to 50. The two
original branches' mutation, audit, timing and campaign records above remain
historical evidence; they are not fresh campaign results for the merged tree.
The accepted loader's only storage-audit exception is the **40 measured
seed-dependent messages, pending Python determinism fix**.

The integrated native loader retains database scalars through its raw admission
path and shares downstream stages with the existing typed assembler. The latter
still has its strict typed conversion boundary; closing review-workflow
lossless/refusal gaps remains separate work before routing. See the
[transfers-14 handoff](rust-shell.md#accepted-loader-integration-r5-transfers-14)
for fresh verification and remaining ordered work. Python retains product
authority. Integrated campaign/timing acceptance and the R4 authority switch
are not established by importing the original branches' evidence.

## R4-13 integration contract

R4-13 must open through `crates/store`, hold `BEGIN IMMEDIATE` and its build
savepoint, run R4-10 repairs/pricing, then call the loader on that same connection
with the exact ordered profile and injected `as_of`. It must consume the eventual
lossless typed snapshot directly, roll back refusals/panics without refreshing
freshness markers, and commit only after successful persistence. CLI/daemon/AI
routes must not call this test-only prototype or adopt its serializer as a
production boundary. Accounting admission and watch-only/no-egress rules remain.
