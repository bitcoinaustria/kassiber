# R4-12 journal persistence checkpoint

R4-12 has an unregistered native persistence service and complete logical-dump
oracles. **Acceptance remains open.** The performance budget, exhaustive
persistence-branch mutation coverage and inherited full R4 campaigns must close
before this becomes production authority. Python remains the reference. No CLI
or daemon registration changes are included.

## Accepted CPU budget for the next continuation

The owner accepts **borrowed read floor + 0.15 process CPU seconds for unchanged
cells**, and **prepared DML write floor + 0.15 for fresh cells**. This supersedes
the proposed-budget language in the historical probes below. Repeat the read
floor over at least five independent connections with load averages and use its
minimum. The implementation and all acceptance gates remain open.

## Read-floor repeat on the requested continuation

Five independent newly opened connections per cell visited every retained
column, with unmasked complete dumps, integrity and foreign keys checked before
and after. Independent process-CPU minima are **0.118601 / 0.213804 seconds**
for generic SQLite / SQLCipher and **0.103358 / 0.240243 seconds** for
multi-wallet SQLite / SQLCipher. The accepted unchanged allowances are therefore
0.268601 / 0.363804 and 0.253358 / 0.390243 seconds for this repeat.

The host remained busy, with one-minute load 41.27–45.72 during these scans;
these are contention-qualified measurements. A quieter window should replace
this repeat if one becomes available during the continuation. The baseline
unchanged generic SQLite writer took 0.525804 CPU seconds in one profiling pair:
entry preparation 0.132154, entry comparison 0.110038, posting preparation
0.060567 and posting comparison 0.141588. Its complete timed dumps match.
This profile locates work and does not establish five-pair acceptance.

[The continuation record](r4-journal-persistence-direct-comparison-measurements.json)
binds every floor trial, load average, book manifest, source and binary hash,
complete-dump hash and the separate baseline profile. No acceptance is claimed.

## Direct comparison continuation (withdrawn)

**R4-12 remains unaccepted.** Four generic SQLite comparison variants matched
all **96 timed complete dumps**, but neither required cell passed its CPU
budget. Their archived sources and binaries remain under worktree `target/`,
with SHA-256 manifests, every timing sample and phase profile in
[the continuation record](r4-journal-persistence-direct-comparison-measurements.json).
Production persistence sources are restored to `51bd20498`; the accepted
budget decision and a mixed-clock removal regression remain.

Ordinary decimal comparisons parsed retained digits into a `u128` coefficient,
checked sign and scale, and compared REAL values including negative zero.
Compact calculations with up to eight fragments compared directly with retained
bytes; larger dictionaries used the earlier validated shape path. Final decimal
bindings were prepared only for changed rows. A wide coefficient still used the
canonical formatter, so this probe does not satisfy a universal no-formatting
contract for unchanged decimals. Existing chronological and posting-id indexes
fed an ordered comparison view, with renderer order retained for inserts.
The stream's unmatched rows used identity lookup so removing an earlier-created
sibling could not accidentally rewrite a surviving row.

The first scan ordered entries by timestamp and id. SQLite added a temporary
sort; following the existing timestamp/creation/id index removed it. Posting
reads followed the existing id index. Further probes reduced the changed-binding
buffer, removed the per-timestamp map and borrowed metadata/calculation text.
These were experiments, not accepted production changes. No schema, persisted
digest, Python reference, dependency or command registration changed.

The closest five-pair variant measured **0.290347 CPU seconds unchanged** against
0.098731 + 0.15 = **0.248731**, missing by **0.041616**. Fresh measured
**0.847164** against its 0.645101 + 0.15 write-floor allowance, missing by
**0.052063**. Independent minima are used throughout. That run's one-minute
load was 20.277–22.452. Later variants ran at load 31.431–57.866 and did not
establish a controlled improvement. No other 20k, appended/changed or 2k cell
was run after these required cells failed.

A second five-connection read-floor repeat ran after compilation. Across both
repeats the independent minima are generic **0.098731 / 0.208712 seconds** and
multi-wallet **0.103358 / 0.225609**, SQLite / SQLCipher respectively. All complete
dumps, integrity and foreign-key checks pass. Read-floor load spans
37.93–45.72; no quiet-host acceptance is claimed. A later CPU-window sample
also found both threads of every physical core busy. The raw loads and CPU
window remain in the record; a genuinely quiet repeat remains outstanding.

The new regression requires a removed sibling with an earlier creation clock
to leave the surviving entry's rowid, every content column and creation time
unchanged, in both identity orders. This pins the fallback needed when retained
creation order differs from identity order. Final verification and the targeted
mutation result are recorded below; the full mutation and inherited campaign
gates remain open.

The last prototype passes 61 journal tests, including complete dumps, refusal
and unwind rollback and physical crash recovery; 59 Python storage/oracle tests
and release all-target clippy with warnings denied pass. The retained removal
regression kills `JP-removed-mixed-clock-identity` at runtime against the restored
writer, and the restored source passes again. The catalog now has 182 recipes;
only this new targeted kill was run here. No exhaustive branch audit or full
mutation campaign is claimed.

Restored-source verification passes 58 journal tests, the read-floor example's
test, 59 Python storage/oracle tests, release all-target clippy with warnings
denied, workspace formatting and applicability checks for all 182 recipes. The
required classifier update produces no generated diff; preflight `--ci` passes
57 tests. Manual second review confirms production-source identity, archive
hashes and the limited evidence scope. Verification build trees are removed;
archives, logs and disposable books remain under `target/`. All launched test
and benchmark jobs have completed. These results do not close any acceptance
gate listed below.

Remaining work, in order:

1. Meet the accepted budgets in all eight 20k cells over five alternating pairs;
   verify appended/changed and 2k comparisons on both shapes and drivers, with
   exact dumps and the signed-zero rewrite rule. Remove the remaining wide
   coefficient formatting from any future no-formatting comparison.
2. Complete the exhaustive persistence branch-to-mutation audit and rerun final
   kills. A targeted regression kill does not replace that campaign.
3. Run resumable 24,000-book and 1,030-permutation campaigns, all retained
   recordings and permutations plus fresh recordings, full census/shadow with
   the known Python shadow read-budget repro, and 2,285 custody mutations.
4. Verify the integrated R4-13 transaction, rollback, settings, visibility,
   response/checkpoint, dropped-handle and post-COMMIT error contracts on both
   physical drivers, then write acceptance.

## Typed comparison probe from `a06bc004f` (withdrawn)

This attempt leaves **R4-12 unaccepted**. The typed comparison prototype was
withdrawn after its generic SQLite unchanged cell missed both the original
write-floor allowance and the proposed read-floor allowance. All final
production persistence sources remain at `51bd20498`. The read-floor probe
below and a new integrated negative-zero regression remain.

The prototype compared borrowed `ValueRef` cells directly with typed metadata,
quantities and decimal coefficients. It checked exact fixed-point spelling and
all five REAL sign bits without preparing ordinary unchanged decimal strings.
Validated calculation fields were compared against the retained compact bytes;
no canonical digest or intermediate JSON tree was built. Source dictionaries
retained canonical first-seen deduplication, with cached ASCII quoting checks
and a bounded small-dictionary comparison before the indexed large path.
Dynamic shapes retained the existing legacy encoding and refusal path.

Final bindings and ordinary compact strings were prepared only for changed
entries and their changed timestamp groups. Fresh books bypassed comparison
preparation once the retained entry table was known empty. Quantity postings
compared cheap amount/state cells first. Preparation, comparison, writes and
COMMIT all stayed inside the native timer. No schema, stored digest, Python
reference, dependencies or command registration changed.

The first exploratory fresh/unchanged pair matched all six timed dumps but
missed unchanged at 0.421432 CPU seconds. The optimized prototype then ran five
alternating pairs per mode, matching all **30 timed dumps**. The table uses
independent process-CPU minima; every sample and phase profile is retained in
[the typed-probe measurement record](r4-journal-persistence-typed-probe-measurements.json).

| Generic SQLite 20k | Python CPU | Native CPU | Write floor CPU | Native − write floor | Result |
| --- | ---: | ---: | ---: | ---: | --- |
| fresh | 1.308421 | 0.842522 | 0.722667 | 0.119855 | Within write floor + 0.15 |
| unchanged | 1.004621 | 0.356593 | 0.000875 | 0.355718 | Misses both allowances |

The unchanged borrowed-read floor is 0.083712 CPU seconds. Native therefore
adds **0.272881 seconds**, exceeding the proposed 0.15 allowance by **0.122881**.
The five-pair load range is 25.023–32.441; this is not quiet-host acceptance.
Preparation and retained comparisons still dominate. These runs do not establish
a controlled speed improvement over the earlier variants, and no final native
optimization is retained. The other 20k cells, appended/changed and 2k cases
were not rerun for this prototype after this required cell failed.

Before withdrawal the prototype passed 60 journal tests, including complete
dumps, refusal/unwind rollback and physical crash recovery. Focused tests also
pinned exact decimal spelling, canonical string escapes, the 4,096-source
calculation dictionary, every typed retained content column and the absence of
final bindings for unchanged rows. Release clippy, all targets with warnings
denied, passed. Its four relocated mutation recipes passed 181-recipe
applicability; no kill rerun or exhaustive branch audit is claimed.
The archived sources, patch, binary and SHA-256 manifest remain under
`target/typed-prototype/`, bound by the measurement record.

The retained `typed_entry_negative_zero_forces_rewrites_in_all_five_real_columns`
test runs two complete stores for each REAL field and requires actual journal
writes on the second store. It checks the exact target entry's new creation
time and excludes sibling rewrites that could mask a missed sign check. It
passes against the restored implementation and kills the existing
`JP-signed-zero` mutation at runtime; the restored source passes again. This is
one targeted kill, not the full mutation campaign or branch audit.

Restored-source verification passes 57 journal tests, the read-floor example's
test, eight Python oracle tests, release all-target clippy with warnings denied,
and workspace formatting. The required classifier update produces no generated
diff, and preflight `--ci` passes 57 tests. A manual second review confirms
production-source identity and the evidence's limited scope. Verification logs,
source hashes and artifact cleanup are recorded in the measurement record.

Remaining acceptance work, in order:

1. Repeat the independent-connection read floor on a quiet host and settle the
   proposed read-floor + 0.15 CPU budget for unchanged cells. Fresh keeps the
   write-floor + 0.15 budget.
2. Implement a cheaper comparison and meet all eight 20k fresh/unchanged cells
   over five alternating pairs, plus appended/changed and 2k comparisons on
   both shapes and drivers, with exact dumps.
3. Complete the exhaustive persistence branch-to-mutation audit and final kills.
   The full 181-recipe campaign has not been rerun here; only the targeted
   `JP-signed-zero` kill above was rerun.
4. Run resumable campaigns for 24,000 books and 1,030 permutations; 159 retained
   recordings and permutations plus fresh recordings; full census and shadow,
   preserving the known Python shadow read-budget repro; and 2,285 custody
   mutations. None ran in this attempt.
5. Verify the integrated R4-13 contract on both physical drivers: outer
   transaction before snapshots/repairs, whole pre-COMMIT rollback, settings
   restoration, committed visibility, response flush before checkpoint, dropped
   handles and post-COMMIT failures without a second failure response. Then
   write acceptance. This attempt does not close that gate.

## Borrowed retained-row read floor from `d6e46912b`

R4-12 remains **unaccepted**. The unregistered `journal-read-floor` example
prepares the same profile-scoped SELECTs as persistence, then visits every
column of every retained entry and quantity posting through borrowed
`rusqlite::types::ValueRef`. Each scalar is touched once through `black_box`;
text and BLOB payloads remain borrowed. It records row, column, type and payload
byte counts. It does no canonical hashing, JSON decoding, string conversion,
row copying or value comparison. This is a lower bound on reading, rather than
a full comparison or a substitute for the DML floor.

Each cell is the minimum process CPU time over five independent, newly opened
connections, matching the native benchmark's decrypted-page cache state.
Query iteration and COMMIT are timed. Opening, statement preparation, complete
dumps and checkpointing stay outside the timer. Every retained column,
including `created_at`, is visited. Plaintext uses WAL/NORMAL with mmap;
SQLCipher uses DELETE/FULL without mmap. Both use the scoped 128 MiB cache.

| 20k book / driver | Entries | Postings | Read floor CPU | Current write floor CPU | Current unchanged allowance |
| --- | ---: | ---: | ---: | ---: | ---: |
| generic / SQLite | 20,000 | 40,000 | 0.083712 | 0.000815 | 0.150815 |
| generic / SQLCipher | 20,000 | 40,000 | 0.216724 | 0.003914 | 0.153914 |
| multi-wallet / SQLite | 19,983 | 45,216 | 0.097404 | 0.011076 | 0.161076 |
| multi-wallet / SQLCipher | 19,983 | 45,216 | 0.237027 | 0.019525 | 0.169525 |

Both SQLCipher read floors exceed their current allowances before preparation
or comparison: by 0.062810 and 0.067502 CPU seconds. Under the benchmark's
fresh-connection conditions, the measured borrowed scan alone cannot meet the
old allowance. **Propose read floor + 0.15 CPU seconds for all unchanged cells;
retain write floor + 0.15 for fresh cells.** The proposed unchanged allowances
are 0.233712 / 0.366724 seconds generic and 0.247404 / 0.387027 multi-wallet,
on SQLite / SQLCipher respectively. These measurements do not establish that
the complete writer meets either budget. No acceptance is claimed.

A first exploratory probe reused one connection for all five repetitions.
Its warm-cache minima (0.085079–0.119118 seconds) did not match the native
benchmark's connection lifecycle. They are retained separately in the record
and excluded from the budget decision. No schema or persisted digest is added.

The host was busy: every sample records `/proc/loadavg`, with one-minute load
31.08–33.63. These are best-of-five process-CPU measurements under contention,
not quiet-host acceptance; the requested quiet-host repeat remains open.
The copied synthetic books retain their previously normalized journals and
matching CSV SHA-256 values. Unmasked complete dumps match before and after
all four cells; integrity and foreign-key checks pass. A focused unit test pins
all five SQLite scalar classes and profile filtering. Offline release build,
that test and example clippy with warnings denied pass.

The [measurement record](r4-journal-persistence-read-floor-measurements.json)
retains all trials, load averages, scalar counts, logical-dump hashes, book
manifests, source and binary hashes, and the separately remeasured unchanged
baseline. Reproduce on disposable normalized books with:

```sh
cargo build --offline --release -p kassiber-journals --example journal-read-floor
uv run --locked python scripts/benchmark-journal-read-floor.py --books target/books/generic-20000 target/books/multi-wallet-20000 --out target/read-floor
```

## Digest probe from `51bd20498` (withdrawn)

This attempt leaves **R4-12 unaccepted**. The schema-preserving digest and lazy
binding prototype passed its correctness checks but missed the CPU budget. It
was withdrawn. Production code, schema, Python reference and mutation recipes
remain at `51bd20498`; this checkpoint adds retained-row regression witnesses
and the [measurement record](r4-journal-persistence-digest-probe-measurements.json).

The clean baseline binary was built and copied before Rust edits. Historical
synthetic books were copied into worktree `target/`, their CSV hashes checked
against the previous continuation, and their journals rebuilt with the current
unchanged Python capture and fixed oracle clock. The first unnormalized copy
failed the unchanged witness with 20,000 entry rewrites; its measurements are
excluded. The record binds baseline sources to the clean commit and preserves
the different checkout hashes that the harness observed during later runs.

All eight normalized baseline fresh/unchanged cells passed complete dumps but
missed the CPU allowance in one exploratory pair each. Host load rose while the
initial build and book preparation ran; the valid baseline samples span
25.333–83.650. These one-pair results establish profiling evidence, rather than
five-pair acceptance or a controlled speed comparison with previous runs.

The prototype hashed every stored content scalar with type and length framing,
excluding the retained write timestamp. Calculations streamed canonical bytes
into SHA-256; unchanged entries skipped final bindings and compact-string
assembly. Decimal conversion and validation remained necessary. Posting digests
were computed during validation. Preparation, comparisons, writes and COMMIT
all remained inside the timer. There was no schema change or persisted digest.

The prototype matched all six stores in its fresh/unchanged generic SQLite pair,
then all fifteen stores in five alternating unchanged pairs. The five-pair
unchanged minimum was **0.400689 CPU seconds**, against a **0.001407-second**
floor: **0.399282 seconds above the floor**, exceeding the allowed 0.15. The
single fresh pair was 0.257360 seconds above its floor. Scalar preparation,
canonical calculation hashing and retained-row reads still consumed the budget.
The fresh/unchanged floor, native and Python runs total **45 exact timed dumps**
across this attempt, including the 24 baseline stores.

A separate disposable reader probe compared direct reads with `json_array`
packing decoded through either `Vec<Value>` or a streaming serde visitor. Both
packed variants were slower in that probe. Its simplified hashing and float
handling do not establish row-digest parity, durability or an acceptance floor.
The complete withdrawn patch, source archive, binaries and probe stay locally
under `target/`, with hashes and paths in the measurement record.

Three new regression tests pin actual delta writes for every content column in
both large tables, retained clocks, ambiguous adjacent text boundaries,
BLOB versus TEXT, NULL versus empty TEXT, and the negative-zero rewrite rule in
all five entry REAL columns. These witnesses remain after withdrawing the
prototype. The prototype itself passed 54 journal tests, including complete
dumps, refusal/unwind rollback and physical SQLite/SQLCipher crash recovery;
its adjusted 181 mutation recipes only passed applicability checking. No new
mutation-kill or exhaustive branch audit is claimed.

Final checkpoint verification: 56 journal tests and eight Python oracles passed;
release journal clippy (all targets, warnings denied), workspace formatting,
181-recipe applicability and manual second review passed. The required classifier
update produced no diff and the required preflight shard passed all 57 tests.
Logs, source hashes and scope are recorded in the measurement record. The
verification build tree was removed after summarizing its results.

Remaining work, in order:

1. Implement a sufficiently cheap typed-value comparison and meet all eight
   20k floor + 0.15 CPU cells over five alternating pairs. Repeat appended,
   changed and 2k comparisons on both book shapes and drivers. This attempt
   did not rerun appended/changed or 2k comparisons.
2. Complete the exhaustive persistence branch-to-mutation audit and kill its
   recipes against the final comparison path. The inherited 181 kills were
   not rerun in this attempt.
3. Run the inherited 24,000 generated books, 1,030 permutations, 159 retained
   recordings and their permutations plus fresh recordings, full census and
   shadow, and 2,285 custody mutations in resumable shards. Preserve the known
   Python shadow read-budget repro below. None ran in this attempt.
4. Verify the integrated R4-13 contract on both physical drivers: whole outer
   transaction ownership before snapshots/repairs, pre-COMMIT rollback,
   setting restoration, committed visibility, response flush before checkpoint,
   dropped handles and post-COMMIT failures without a second failure reply.
   Then write the acceptance record. No integrated R4-13 witness ran here.

## Continuation from `32767d331`

This continuation keeps **R4-12 unaccepted**. It reduces preparation without
changing the schema, stored values, UUIDs, DML order, transaction policy or
the public persistence stage. No preparation moves outside the persistence
timer. Unchanged entries still require calculation/scalar preparation and full
retained-row comparison; the required cheap unchanged path remains open.

Entry bindings borrow metadata and pricing text rather than cloning them into
SQL values. Ordinary REAL and exact decimal bindings share one fixed spelling;
long spellings retain the original Decimal conversion. Metadata and amount refusals
still precede decimal formatting, including when both inputs are invalid.
Quantity postings extract their fields once, reuse observation metadata and
borrow balance keys. The additional `quantity_postings_bind` span separates
the observation index from posting preparation.

Calculation-source encodings are shared within one store. Cache keys bind the
immutable borrowed provenance owner and every independent source operand,
including the exact fee spelling. The borrow lifetime prevents provenance
mutation or address reuse while cached. Per-calculation dictionaries still
deduplicate by encoded content in first-seen order, and invalid shapes retain
the legacy fallback. The cache is dropped on return, refusal or unwind.

### Appended benchmark correction

The inherited benchmark copied a transaction's original `raw_json.txid` while
changing its table id, fingerprint and external id. Those rows can remain
aliases of the original observation. An isolated generic-2k reproduction adds
20 transaction rows but produces **zero appended entries**, with 2,000 entries
and 4,000 postings unchanged. Giving those rows distinct raw txids and dates
produces 2,020 entries and 4,040 postings. This is a benchmark fixture defect;
Python product code is unchanged.

The corrected harness appends after the latest observation, within the injected
build clock, and updates raw identity/date and a present confirmation timestamp.
It records entry/posting INSERT and DELETE activity and refuses vacuous fresh,
unchanged, appended or changed cases. Every appended transaction must reach its
entry. Earlier appended timing rows lacked this witness and are superseded.
The benchmark now records its own source hash as well as native source hashes.

### Continuation measurements

A clean-base generic-20k profile ran before editing. Its unchanged store took
0.459113 CPU seconds: entry preparation 0.113357, entry comparison 0.062483,
posting preparation 0.100754 and posting comparison 0.110475. That single pair
identifies the work to remove; it is not a five-pair acceptance result.

The final variant ran all four rebuild modes on both book sizes and drivers:
**480 timed stores, all exact dumps**. Each table cell below is an independent
minimum over five alternating Python/native pairs, with a floor replay after
each pair. Units are process CPU seconds; every wall sample and phase profile
is retained in the [continuation measurement record](r4-journal-persistence-continuation-measurements.json).
Load ranges include all three engines' samples. Preparation, retained-row
comparison, writes and COMMIT stay timed. Decoding, rendering, dump verification
and passive checkpointing stay outside the timer.

Only generic SQLite fresh meets the required 20k floor + 0.15 CPU budget;
**seven of eight fresh/unchanged cells fail**. All unchanged 20k cells fail.
Native beats Python in every measured mode, but R4-12 remains unaccepted.
All 2k modes are within 0.047 CPU seconds of the floor. These comparisons do
not establish a speed improvement over the historical variants under different
host contention. A cheap, fully validated unchanged-row path remains necessary.

20,000 transactions:

| Book / rebuild | Python CPU | Native CPU | Floor CPU | Native − floor | Load 1m range |
| --- | ---: | ---: | ---: | ---: | --- |
| generic-sqlite / fresh | 1.504052 | 0.931166 | 0.853787 | 0.077379 | 42.941–46.548 |
| generic-sqlite / unchanged | 2.158688 | 0.769648 | 0.001453 | 0.768195 | 52.110–62.775 |
| generic-sqlite / appended | 2.342143 | 0.802534 | 0.028169 | 0.774365 | 53.500–59.256 |
| generic-sqlite / changed | 1.434473 | 0.669721 | 0.148010 | 0.521711 | 43.791–62.193 |
| multi-wallet-sqlite / fresh | 1.552594 | 1.116595 | 0.951404 | 0.165191 | 31.765–36.177 |
| multi-wallet-sqlite / unchanged | 1.432145 | 0.453280 | 0.014439 | 0.438841 | 34.413–45.429 |
| multi-wallet-sqlite / appended | 1.482188 | 0.466169 | 0.029273 | 0.436896 | 41.725–46.224 |
| multi-wallet-sqlite / changed | 1.867898 | 0.585272 | 0.188936 | 0.396336 | 33.721–36.030 |
| generic-sqlcipher / fresh | 1.900272 | 1.004052 | 0.774469 | 0.229583 | 29.255–33.992 |
| generic-sqlcipher / unchanged | 2.556472 | 0.957430 | 0.004577 | 0.952853 | 36.122–57.454 |
| generic-sqlcipher / appended | 1.638017 | 0.599036 | 0.072553 | 0.526483 | 47.216–58.188 |
| generic-sqlcipher / changed | 1.692202 | 0.923522 | 0.409398 | 0.514124 | 26.570–30.851 |
| multi-wallet-sqlcipher / fresh | 1.736885 | 1.118850 | 0.844563 | 0.274287 | 25.150–30.489 |
| multi-wallet-sqlcipher / unchanged | 1.446978 | 0.608311 | 0.019554 | 0.588757 | 24.261–29.088 |
| multi-wallet-sqlcipher / appended | 1.495798 | 0.708857 | 0.088449 | 0.620408 | 24.616–26.249 |
| multi-wallet-sqlcipher / changed | 1.942433 | 1.114866 | 0.482496 | 0.632370 | 21.479–22.852 |

2,000 transactions:

| Book / rebuild | Python CPU | Native CPU | Floor CPU | Native − floor | Load 1m range |
| --- | ---: | ---: | ---: | ---: | --- |
| generic-sqlite / fresh | 0.112811 | 0.071574 | 0.057841 | 0.013733 | 21.282–22.090 |
| generic-sqlite / unchanged | 0.077541 | 0.028323 | 0.000930 | 0.027393 | 20.716–20.779 |
| generic-sqlite / appended | 0.088368 | 0.031780 | 0.003146 | 0.028634 | 20.258–21.679 |
| generic-sqlite / changed | 0.110395 | 0.041262 | 0.013376 | 0.027886 | 21.144–21.773 |
| multi-wallet-sqlite / fresh | 0.134229 | 0.088138 | 0.071129 | 0.017009 | 20.738–21.151 |
| multi-wallet-sqlite / unchanged | 0.109622 | 0.041821 | 0.006241 | 0.035580 | 20.894–21.320 |
| multi-wallet-sqlite / appended | 0.116552 | 0.038112 | 0.008513 | 0.029599 | 20.375–20.582 |
| multi-wallet-sqlite / changed | 0.150519 | 0.049409 | 0.020427 | 0.028982 | 20.948–21.466 |
| generic-sqlcipher / fresh | 0.156806 | 0.085447 | 0.074710 | 0.010737 | 23.208–27.196 |
| generic-sqlcipher / unchanged | 0.112926 | 0.046507 | 0.002907 | 0.043600 | 21.324–22.481 |
| generic-sqlcipher / appended | 0.112525 | 0.056402 | 0.014135 | 0.042267 | 21.503–22.824 |
| generic-sqlcipher / changed | 0.147470 | 0.088989 | 0.047962 | 0.041027 | 20.192–22.900 |
| multi-wallet-sqlcipher / fresh | 0.159128 | 0.105484 | 0.081055 | 0.024429 | 22.663–23.592 |
| multi-wallet-sqlcipher / unchanged | 0.123538 | 0.057623 | 0.011377 | 0.046246 | 22.312–23.168 |
| multi-wallet-sqlcipher / appended | 0.143286 | 0.070378 | 0.023448 | 0.046930 | 22.595–23.430 |
| multi-wallet-sqlcipher / changed | 0.195903 | 0.104858 | 0.059261 | 0.045597 | 22.994–24.300 |

### Continuation verification and stop state

| Check | Final result |
| --- | --- |
| Five alternating pairs, all modes/sizes/drivers | 480 exact timed-store dumps |
| Offline release journal tests, one test thread | 53 passed; zero failed or ignored |
| Offline release journal clippy, all targets, `-D warnings` | Pass |
| Workspace formatting check | Pass |
| Python render/storage oracles | 8 passed |
| Journal mutation catalog | 181/181 killed; zero survivors or build errors |
| Unmutated mutation baselines | 35 distinct filters passed across three shards |
| Required test-classifier update | Complete; no generated diff |
| Required preflight shard, `--ci` | 57 passed |
| Manual second review | Complete |

The journal tests include complete dumps, error/unwind rollback and deterministic
process-termination recovery on physical SQLite WAL/NORMAL and SQLCipher
DELETE/FULL books. These are the existing persistence-stage witnesses, with the
new refusal-order case; they do not exercise the R4-13 caller's repairs.

The mutation run is resumable by source hashes, with incremental per-shard logs
and explicit recipe subsets in the measurement record. It runs the 17 new
recipes followed by the inherited 164, partitioned into 64/64/53. Each shard
deletes its build tree before the next starts. One initial decimal-sign recipe
failed to compile because method-call syntax prevented float type inference;
it was corrected to `f64::abs(float)` and the entire shard rerun. That initial
build error is recorded separately and is not counted as a kill. Production
source did not change during the timing or final verification campaigns.

This is a **verified partial checkpoint**, not R4-12 acceptance. What remains:

1. Implement the cheap unchanged-row path and meet the final 20k CPU budget on
   both book shapes and drivers. Three fresh and all four unchanged cells still
   fail; repeat the comparisons after further production changes.
2. Finish the exhaustive persistence branch-to-mutation audit. The 181 named
   kills do not establish exhaustive coverage.
3. Rerun the inherited 24,000 books, 1,030 permutations, 159 recordings at
   `/home/tg/.cache/kassiber-verify/custody-recordings/` and their permutations
   plus fresh recordings, full census/shadow, and 2,285 custody mutations.
   None of those full campaigns was rerun in this continuation. Preserve the
   known Python shadow read-budget repro below.
4. Execute the integrated R4-13 contract witnesses below on both physical
   drivers, including post-COMMIT failure handling, then write the acceptance
   record. The source review below does not close that gate.

### R4-13 caller contract review

The inspected `t3code/r4-13-journals-process-2` revision is `16c145721`.
This is a source review, **not an integrated verification result**. Its
`process::process` currently acquires `BEGIN IMMEDIATE` before profile
resolution, repairs, pricing and snapshot loading. Its transaction guard owns
the whole rollback; the persistence stage's nested savepoint cannot undo
earlier repairs. The branch has named tests for pre-build refusals, phase
unwinds, persistence failure after repairs and successful atomic persistence.
Those tests have not been rerun with this continuation.

The integration must preserve these boundaries:

- Hold the borrowed profile, custody, wallet and ledger owners until persistence
  returns. The source-encoding cache lives only within that call.
- Enter the connection cache and checkpoint guards while idle, acquire the
  write transaction before any snapshot or repair, and keep every repair,
  pricing write, journal write and freshness marker in the same transaction.
- On any pre-COMMIT refusal or unwind, roll back the entire transaction,
  including pre-build writes, and restore the connection settings. A caller
  that catches the persistence refusal must still perform this outer rollback.
- Publish success only after COMMIT. Restore the rebuild cache after transaction
  completion; retain the checkpoint guard until the response is published and
  flushed. Only then invoke `after_reply`, which performs PASSIVE on WAL and
  restores the previous automatic-checkpoint setting. SQLCipher stays
  DELETE/FULL and receives no WAL checkpoint.
- Treat errors after COMMIT separately from rebuild failures: they cannot
  roll back committed repairs or journals. The inspected orchestration still
  has a fallible cache restoration and a progress callback after COMMIT; its
  probe also lets an `after_reply` error reach the outer error-response path.
  The integrated transport needs witnesses for these cases, including no
  second failure response after publishing success.

Final verification must exercise both physical drivers, pre-build and
persistence error/unwind paths, success visibility from another connection,
response flush before checkpoint, and setting restoration after a dropped
response/checkpoint handle. The existing persistence-only rollback and crash
witnesses do not establish the complete caller contract.

## Follow-up from `b5eb38ea8`

This local follow-up improves persistence and adds semantic and crash witnesses.
**R4-12 is still unaccepted:** the process-CPU budget fails, the exhaustive
branch audit remains open, and the inherited full R4 campaigns remain open.
The evidence below separates this run from the starting checkpoint.

### Implementation and stage contract

The native path now encodes typed calculations directly into the exact compact
bytes, binds quantity-posting text by reference, and indexes changed rows by
position. Entry UUID preimages use the ASCII tuple directly. Source dictionaries
retain first-seen order with indexed deduplication, including thousands of
distinct sources in one calculation. Dynamic shapes retain the existing legacy
fallback. Statements remain prepared once per table; rows execute in reference
order with the same indexes and transaction policy.

Economic-relation fallback amounts now use the custody service's exact principal
primitive, including fee treatment. Missing source identifiers use Python's
first-truthy-id-or-empty-string rule. Filed-chain authority and report barriers
have additional witnesses. Cache scopes disarm successful explicit restoration
and retain signed nested restoration on refusal and unwind. The prerequisite
restack repairs remove duplicate manifest keys and adapt the direct loader to
the current snapshot row type; no dependency versions or schemas change.

The public stage remains
`persistence::store_ledger_state(conn, profile, &JournalState, clock, ids, observer)`.
It returns `Stored` with the processing marker, counts, quarantines and resolved
impact count. Its inner savepoint rolls back errors and unwinds. `phase` is an
optional default callback on `WriteObserver`; the service reads no clock.
R4-13 must retain the borrowed stage owners, enter the cache/checkpoint guards,
acquire `BEGIN IMMEDIATE` before snapshot reads and repairs, own the outer
savepoint and COMMIT, publish success after COMMIT, and run `after_reply` after
the response. Error handling must unwind pre-build repairs as well as this
stage. The writer consumes the shared plaintext WAL/NORMAL or SQLCipher
DELETE/FULL connection policy.

The R4-13 continuation adds `store_ledger_state_with_clock`, which accepts
the stored-row timestamp and a fallible clock callback separately. It samples
the custody-impact service timestamp once, even with no pending impacts, and
each chain-impact resolution timestamp independently; `rebuilt_at` remains the
stored-row timestamp. The original entry wraps it with a constant callback.
The stage still has no implicit clock or commit. Full-process clock/UUID tapes
and late-clock rollback witnesses are recorded in the
[R4-13 checkpoint](r4-journal-process-checkpoint.md).

### CPU and wall measurements

Each cell is **process CPU / wall seconds**, with independent minima over five
alternating Python/native pairs. Floor, native and Python each pass an unmasked
complete dump comparison in every trial: 120 timed stores verified. The floor
prepares statements and converts recorded scalars before timing; execution and
COMMIT remain timed. Native calculation/scalar preparation and comparison remain
inside its timer. Decoding, rendering, dump verification and the passive
checkpoint stay outside. Both book kinds have 20,000 transactions; generic has
20,000 entries, multi-wallet has 19,983 entries and 17 quarantines.

| Book / rebuild | Python CPU / wall | Native CPU / wall | Floor CPU / wall | Native CPU − floor | Load 1m range |
| --- | ---: | ---: | ---: | ---: | --- |
| generic-sqlite / fresh | 1.758730 / 1.866601 | 1.080039 / 1.137810 | 0.740874 / 0.859770 | 0.339165 | 65.882–90.029 |
| generic-sqlite / unchanged | 1.280150 / 1.299705 | 0.502755 / 0.559595 | 0.002157 / 0.017326 | 0.500598 | 51.538–57.471 |
| multi-wallet-sqlite / fresh | 1.978529 / 2.156990 | 1.374834 / 1.443026 | 0.877888 / 0.948535 | 0.496946 | 41.404–47.454 |
| multi-wallet-sqlite / unchanged | 1.297317 / 1.310028 | 0.531179 / 0.573347 | 0.012889 / 0.045795 | 0.518290 | 37.172–45.206 |
| generic-sqlcipher / fresh | 1.725692 / 1.870674 | 1.119964 / 1.304450 | 0.712771 / 0.886034 | 0.407193 | 43.472–50.606 |
| generic-sqlcipher / unchanged | 2.291301 / 2.577306 | 0.962341 / 1.176505 | 0.004416 / 0.093124 | 0.957925 | 64.789–78.438 |
| multi-wallet-sqlcipher / fresh | 2.394335 / 2.733182 | 1.335984 / 1.487191 | 1.129136 / 1.398471 | 0.206848 | 59.610–71.390 |
| multi-wallet-sqlcipher / unchanged | 1.858559 / 1.960311 | 0.760920 / 0.840818 | 0.020987 / 0.096197 | 0.739933 | 50.828–61.363 |

All eight modes miss **floor + 0.15 CPU seconds**. Phase profiles identify
calculation preparation, posting preparation and retained-row comparison as
remaining overhead. The direct typed encoder removes the intermediate JSON
object tree; borrowed postings and indexed change flags remove repeated string
copies and ID bookkeeping. SQL index maintenance remains included in both the
native run and its exact DML floor. SQLCipher retains DELETE/FULL and the scoped
128 MiB cache required for coexistence; durability is unchanged.

[The follow-up measurement record](r4-journal-persistence-followup-measurements.json)
retains every CPU/wall/load trial, phase measurements, binary/source hashes,
book seed and CSV hashes, plus separate final verification hashes and results.
These measurements precede the final indexed source
dictionary and missing-ID hardening; their source hashes identify the measured
variant. They do not claim a final-revision timing pass. The earlier exploratory
before/after profile mixed encoder revisions in its unchanged case and is not
an acceptance baseline. Appended and changed rebuilds were not remeasured here.

### Follow-up verification

The new process-termination campaign flushes dirty pages after each successful
DML, handshakes with its parent, and is killed at each such boundary, immediately
before COMMIT, and immediately after COMMIT. It reopens all cases, checks the
full stale or committed dump, and checks integrity and foreign keys. It covers
three fixtures (cipher, filed-change and chain retraction) on physical SQLite
WAL/NORMAL and SQLCipher DELETE/FULL files. This is deterministic termination
between DML statements, rather than instruction-level fault injection inside
SQLite's COMMIT implementation.
The helper's self-launch is reviewed by exact file, function and call line in
`crates/net/tests/reviewed-process-calls.txt`; the process guard passes all nine
tests with that entry.

Named witnesses additionally pin malformed bindings and holding tuples,
compact-shape refusals, marker overflow, missing/inactive/revised reconciliation
components, relation fallbacks/nonpositive amounts, unavailable or unconfirmed
chain authority, missing filed snapshots, component barriers, and signed cache
and checkpoint restoration. The refreshed Python-generated storage vectors
retain source/pricing/fragment missing-key and unknown-key cases.

| Follow-up check | Result |
| --- | --- |
| `cargo fmt --all -- --check` | Pass |
| Offline release workspace clippy, all targets, `-D warnings` | Pass |
| Offline release workspace tests, one test thread | 1,648 passed; zero failed or ignored |
| Offline release journal tests, one test thread | 50 passed; zero failed or ignored |
| Python render and storage oracles | 8 passed |
| Expanded journal mutation catalog | 164/164 killed; 32 baseline filters passed; zero survivors or build errors |
| Final mutation catalog validation | 164 recipes valid against the final source |
| Process/network chokepoint guard | 9 passed |
| Required test classifier update | Complete; refreshed inherited subprocess flags and generated counts |
| Required preflight shard | 57 passed |
| Manual second review | Complete; bounded deduplication and missing-ID cases fixed |

The preflight initially found an inherited documentation command with its locked
flag out of the required position. Reordering those flags closes that failure;
Python product behavior is unchanged. The complete workspace run used the local
mock-socket sandbox exception; no test remains blocked by the sandbox. The
mutation campaign preceded the final test-only module relocation and signed-zero
witness; final source validation and the workspace run cover those changes.
The mutation build tree was removed automatically. Verification build artifacts
are removed after their logs and source hashes are summarized.

### Remaining acceptance work and integration handoff

1. Meet floor + 0.15 process CPU seconds on final-revision generic and
   multi-wallet 20k, on SQLite and SQLCipher, with exact dumps; repeat appended
   and changed rebuilds and the inherited 2k comparisons.
2. Finish the branch-to-mutation audit beyond the expanded catalog. Named
   failures establish each recipe's witness, not exhaustive coverage by count.
3. Rerun the inherited 24,000 books and 1,030 permutations, 159 recorded requests
   and permutations plus fresh recordings, full census/shadow, and 2,285 custody
   recipes. Preserve the known Python shadow read-budget repro below.
4. Validate the R4-13 caller's whole-transaction ownership, response/checkpoint
   ordering and rollback of pre-build repairs when its integration lands.

The remaining sections retain the **starting checkpoint's historical evidence**;
its 682-match corpus and earlier timing/mutation counts are not rerun claims for
this follow-up.

## Implementation and ownership

`crates/journals/src/persistence.rs::store_ledger_state` consumes the R4-11
`JournalState`, the profile, an injected clock and a caller-supplied UUID source.
It binds SQLite scalars through the existing store driver and prepares each
table's write statement once. The transitional stage-output JSON seam remains;
the ledger response and rendered entries retain native types. No dependency
version changes are required.

The service mirrors compact v2 calculation bytes, UUIDv8 entry ids, column-tuple
comparison excluding `created_at`, timestamp-group rewrites, signed-zero REAL
comparison, unchanged rowid retention and quantity-posting deltas. Small derived
tables are replaced. An absent quantity stage preserves its old quantity tables
and still clears reconciliation proofs, following Python. Quarantine
deduplication retains Python's first-seen order,
root precedence, carried reasons, nonfinite legacy JSON and escaped surrogates;
storage filters synthetic transaction ids and enriches live ownership candidates.
Holdings, exact prices, freshness counts and input versions follow Python.

Processing markers are read from the database and advance by one microsecond
when the injected clock repeats or moves backwards. Row timestamps and filed
resolution timestamps use the original clock. This preserves explanation expiry
without changing unchanged entry timestamps. Filed-impact summaries and chain
authority checks reuse the existing reports and wallets services. Quarantines,
component blockers and quantity issues retain the report-readiness barrier.

The shared journal SQLite refusal conversion now preserves Python DB-API
exception families and original SQLite messages, including trigger
`IntegrityError` and missing-table `OperationalError`; see the
[R4-10 refusal audit](r4-journal-pipeline.md#r4-10-branch-and-cache-checkpoint).
This changes no stored representation or transaction ownership.

The caller owns `BEGIN IMMEDIATE`, the encompassing `journals_process`
savepoint, all pre-build repairs and COMMIT. The writer additionally holds a
rollback-on-error/unwind savepoint. It cannot prove which BEGIN mode a caller
used; R4-13 must acquire the write lock before reading the snapshot and unwind
the outer savepoint on any build or persistence failure.

`storage::RebuildCache` scopes `cache_size=-131072` and restores its exact signed
previous value on success, refusal or unwind. It leaves spill settings intact.
`storage::DeferredCheckpoint` disables automatic WAL checkpoints before BEGIN,
runs PASSIVE after COMMIT and the response, then restores the prior setting.
SQLCipher remains DELETE/FULL and receives no WAL checkpoint. These guards are
caller-owned so they can cover the complete rebuild transaction.

## Starting checkpoint dump oracle and corpora

Captures preserve every table, column, rowid, id and timestamp. Both runtimes
start from the same logical snapshot; benchmark trials copy the same physical
starting database. Nothing is masked. SQL scalar types are preserved and REAL
values are compared by all 64 bits; JSON exponent spelling is transport syntax.
The current schema has 112 tables.

The twelve generator families are sampled once per scenario. Raw-stage
generators can clone an import fingerprint without creating a database. The
harness gives those clones distinct fingerprints **before** stage computation
and database insertion in either runtime. Schema-invalid or pre-persistence
refusals remain separately recorded; they do not count as matched persistence.
The sampled harness materializes storage-relevant scope, observations and
component headers; it supplies the already-computed stage at the boundary.
It does not recreate every raw custody leg or evidence row and therefore is
not a whole-pipeline custody oracle.

| Corpus | Complete dumps matched |
| --- | ---: |
| Twelve sampled generator families | 502 / 502 |
| Separate payout books | 10 / 10 |
| Focused census captures | 69 / 69 |
| Focused custody-shadow captures | 75 / 75 |
| Frozen persistence cases | 26 / 26 |
| Total (frozen cases can overlap captures) | 682 / 682 |

Thirteen sampled generator cases refused at schema validation or upstream of
persistence. Three payout inputs (`legacy_NaN`, `legacy_inf`, `legacy_junk`)
refused in Python before persistence. Neither group is counted as parity.
The 26 frozen cases also pass on physical SQLCipher DELETE/FULL books, including
reopen verification. Fourteen base fixtures are replayed through Python in
`test_rust_journal_persistence.py`; the other twelve retain captured stage
operands. The benchmark adds physical-copy parity for fresh, unchanged,
1% appended and 5% changed journals.

The focused census run passed 99 tests. The focused shadow run passed 128 tests
and two subtests, with one pre-existing read-budget failure:

```sh
KASSIBER_TEST_CUSTODY_ENGINE=shadow uv run --locked python -m pytest \
  'tests/test_custody_native_reconciliation.py::test_disjoint_reviewed_routes_limit_observation_reads[True]' -q
```

This also fails without the persistence capture plugin: shadow executes the
reference work again, giving 5,254 observation reads against the 3,840 budget.
The failure is preserved as a harness/performance repro; Python product code is
unchanged. The broader census/shadow campaigns have not been run here.

## Starting checkpoint verification and measurements

`every_write_failure_and_panic_rolls_back_the_journal` injects both a returned
error and an unwind after each of 604 successful DML statements across the
26 frozen stores: 1,208 rollback cases. The physical cipher case adds 38
injections over its 19 writes, for **1,246 injections**. Complete dumps match
the stale starting database both after the writer's inner rollback and after
the caller rolls back its journal savepoint and transaction. Cache and
checkpoint tests separately pin signed setting restoration, DELETE/FULL,
deferred PASSIVE behavior and a retained WAL reader.

The complete journal mutation catalog kills **112 / 112** recipes: 61 inherited
render/join recipes and 51 persistence recipes, with zero survivors or build
errors. All 20 named baseline filters pass before mutation. Every recipe names
its failing test in `crates/journals/scripts/mutations.json`; a build failure
does not count as a kill. The disposable build tree was removed automatically.
The broader branch audit and inherited custody campaign remain open.

| Check | Result |
| --- | --- |
| `cargo fmt --all -- --check` | Pass |
| Offline workspace clippy, all targets, `-D warnings` | Pass |
| Offline workspace tests, one test thread | 1,510 passed; zero failed or ignored |
| Journal package tests | 24 passed |
| Python render and storage oracles | 8 passed |
| Required inventory update and preflight `--ci` | Updated; 57 passed |
| Capture target guard, empty-corpus refusal and Python syntax | Pass |

The first sandboxed workspace run could not bind local mock sockets in twelve
chain tests. The same complete workspace command passed after sandbox escalation;
no test remains blocked by the sandbox. Preflight refreshed eight generated
error-index source references for the prerequisite storage changes. The focused
shadow read-budget repro above remains separate from these passing gates.

### Starting checkpoint persistence timing

The books were built by `scripts/journals-bench-books.py` at `4f6ca7d06`,
with 20,000 transactions, generic seed 1 and multi-wallet seed 7. Generic
has 20,000 entries and no quarantines. Multi-wallet has 19,983 entries,
17 quarantines and 300 paired pegs. The SQLCipher matrix encrypts a copy of
the generic book using a synthetic test key. No real books are opened.

The timers cover storage and COMMIT. Rendering, request decoding, complete
dump comparison and the deferred checkpoint are outside the timer. The floor
replays Python's exact recorded DML using prepared statements and scalar
values converted before timing; binding, execution and COMMIT remain timed.
It therefore measures the new compact format and changed-row writes rather
than reusing the old-format floor. Python and Rust alternate back to back;
all modes use five pairs because the one-minute load exceeds 8. The minima
for Python, Rust and floor are selected independently. Every measured store
also passes the unmasked dump oracle.

| Book / rebuild | Python (s) | Rust (s) | Floor (s) | Rust − floor (s) | Load 1m range |
| --- | ---: | ---: | ---: | ---: | --- |
| generic / fresh | 1.325212 | 1.389373 | 0.665849 | 0.723524 | 18.099–21.243 |
| generic / unchanged | 1.054679 | 0.988387 | 0.027396 | 0.960991 | 15.845–17.524 |
| generic / appended | 1.725385 | 0.986117 | 0.034397 | 0.951720 | 14.970–27.080 |
| generic / changed | 1.208219 | 1.142653 | 0.187284 | 0.955369 | 19.834–21.690 |
| multi-wallet / fresh | 1.636201 | 1.539973 | 0.837604 | 0.702369 | 17.289–19.870 |
| multi-wallet / unchanged | 1.185058 | 1.038579 | 0.039320 | 0.999259 | 21.156–25.740 |
| multi-wallet / appended | 1.212678 | 1.039325 | 0.032752 | 1.006573 | 20.763–22.720 |
| multi-wallet / changed | 1.336596 | 1.191396 | 0.191914 | 0.999481 | 17.587–18.867 |
| cipher / fresh | 1.613862 | 1.582221 | 0.784623 | 0.797598 | 17.134–17.706 |
| cipher / unchanged | 1.266166 | 1.127561 | 0.070325 | 1.057236 | 15.237–15.624 |
| cipher / appended | 1.244402 | 1.095607 | 0.066551 | 1.029056 | 14.887–16.046 |
| cipher / changed | 1.709245 | 1.551336 | 0.465619 | 1.085717 | 13.916–19.496 |

**The ≤ floor + 0.15 s target fails in every measured mode.** The fresh
plaintext overhead is 0.724 s generic and 0.702 s multi-wallet. Unchanged and
partial rebuilds still prepare and compare complete entry projections. Further
profiling and native calculation/scalar preparation work remain; these results
do not establish which part dominates.

[The measurement record](r4-journal-persistence-measurements.json) retains every
trial, load, source hash, CSV hash and corpus/write summary. Raw captures,
benchmark copies and disposable mutation build trees are removed after their
results are recorded.

## R4-13 handoff and remaining acceptance

- Keep the 18 borrowed fields and their owners alive through persistence. Supply
  Python-equivalent ledger response and completed stage outputs at the existing
  boundary until the native pipeline replaces them.
- Acquire BEGIN IMMEDIATE before reads and repairs; own the outer journal
  savepoint, cache scope, rollback/unwind handling and COMMIT. Do not allow a
  caught storage error to commit pre-build repairs or fresh markers.
- Open books with the shared connection policy: plaintext WAL/NORMAL and
  SQLCipher DELETE/FULL during coexistence. The writer consumes that connection;
  it does not change journal mode inside a pending transaction.
- Use one injected clock for row/impact timestamps and a UUID provider for small
  replacement tables. Entry construction consumes no UUID4 values.
- Publish CLI/daemon success only after COMMIT; invoke the passive checkpoint
  after the reply/response record. Restore connection settings on every exit.
- Preserve report barriers and explanation binding. Do not expose this probe,
  SQL floor replay or arbitrary filesystem/SQL dispatch to AI tools.
- Close the 20k floor-plus-0.15 s budget, exhaustive persistence mutations and
  the inherited 24,000-book/permutation, recording/replay, full census/shadow and
  2,285-recipe custody campaign gates. This checkpoint does not claim them.

The starting checkpoint's named audit gaps included compact-shape validation,
binding widths, reconciliation validity, relation fallbacks, filed-chain authority
and cache/checkpoint restoration. The follow-up above adds witnesses and recipes
across those categories. The remaining audit must map every conditional and
refusal in `persistence.rs`, `persistence/{quantity,filed,quarantine}.rs`,
`storage.rs` and `storage/calculation.rs` to its named killed recipe, documenting
unreachable and driver-error-only paths. Catalog size alone does not establish
that audit. The complete
inherited campaign also includes 1,030 generator permutations, 159 recorded
requests and their permutations plus fresh recordings, and the 2k benchmark
comparisons. None of those full campaigns is replaced by the focused captures.

## Reproduction

Use `MATURIN_NO_INSTALL_RUST=1 mise exec rust@1.99.1 uv@latest --` for commands;
the environment was synchronized with `uv sync --locked --python 3.11`.
Cargo runs offline. Point `TMPDIR` at an existing directory under worktree
`target/`; a read-only shared uv cache can use `UV_NO_SYNC=1` and
`UV_CACHE_DIR=target/uv-cache` after synchronization. Never use a real book.
Use `CARGO_BUILD_JOBS=8`, `CARGO_TARGET_DIR=$PWD/target` and
`CARGO_BUILD_BUILD_DIR=$PWD/target/build` to keep builds bounded and local.

```sh
cargo test --offline -p kassiber-journals -- --test-threads=1
uv run --locked python -m pytest tests/test_rust_journal_persistence.py -q
cargo build --offline --release -p kassiber-journals --example journal-persist
uv run --locked python scripts/capture-journal-persistence-samples.py target/r4-persist/samples
uv run --locked python scripts/capture-journal-persistence-samples.py target/r4-persist/payouts --payouts
uv run --locked python scripts/verify-journal-persistence.py target/r4-persist/samples/*/ target/r4-persist/payouts/payouts --probe target/release/examples/journal-persist --summary target/r4-persist/parity.json
uv run --locked python scripts/benchmark-journal-persistence.py --book target/r4-persist/bench/generic-20000 --out target/r4-persist/timings/generic
uv run --locked python scripts/benchmark-journal-persistence.py --book target/r4-persist/bench/multi-wallet-20000 --out target/r4-persist/timings/multi-wallet
uv run --locked python scripts/benchmark-journal-persistence.py --book target/r4-persist/bench/generic-20000 --out target/r4-persist/timings/cipher --cipher
uv run --locked python crates/journals/scripts/mutation_check.py
cargo test --offline --workspace -- --test-threads=1
uv run --locked python -m tests.rust_port.classify_tests update
uv run --locked python scripts/python_test_shards.py run preflight --ci
```

Build bench books first with `scripts/journals-bench-books.py`, kinds `generic`
and `multi-wallet`, 20,000 transactions. Pairs alternate Python/Rust order and
record one-minute load for each runtime and floor run. At load above 8 use the
best of five. Rendering and dump comparison are outside the timer; persistence
and COMMIT are inside, and the deferred checkpoint is outside. The floor replays
the exact reference DML with statements prepared and scalars converted before timing,
including the new compact format and changed-row behavior.
