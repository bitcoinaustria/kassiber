# R4-9 native custody loader checkpoint

This is an **unaccepted, test-only checkpoint**, dated 2026-10-31. The direct
loader continues `569d80add` (itself based on r7 `dd433e4e` and baseline records).
Generic 20k takes **0.505 s process CPU / 0.509 s wall**, including admission;
loading alone takes 0.465 / 0.468 s. Both miss the 0.30 s target. Python retains
every production load and journal route. R4-9 remains open for re-planning.

## Implemented boundary

`crates/journals/src/loader.rs` exposes `load(connection, profile, as_of)` only
under tests or `test-harness`. The connection belongs to an already opened
`kassiber_store::Book`. Loading requires the caller's SQLite write transaction,
reads pending writes, and never begins, commits, rolls back, repairs or updates
the book. Native tests retain the caller's rollback ownership on success and
refusal, including a pending canary write.

SQL rows now populate today's custody `Snapshot` records directly, using
prepared statements, typed SQLite `ValueRef` matches, preallocated row vectors
and a column layout sorted once per statement. There is no second conversion
of the complete snapshot. `into_custody()` validates and returns that same
structure. The current consumer owns `serde_json::Map<String, Value>` records;
it cannot borrow or intern their keys. Normalized configs use borrowed SQLite
TEXT for memo lookup. Owned-index descriptor derivation batches and caches
targets per canonical descriptor fingerprint and scan horizon within one load.

Column-order metadata and small ordered derived facts support the test-only
Python v1 serializer. Ownership, receipt sources, network references and the
bounded >50k worklist still retain some ordered-value helpers/conversions.
This is not the eventual lossless typed `BookSnapshot` model. INTEGER and REAL
remain distinct; the current adapter still refuses BLOB, invalid UTF-8 TEXT
and nonfinite REAL rather than preserving every legacy `SqlValue`.

The loader reads network facts, wallets/transactions/rates, active components
and children/anchors/revision headers, ownership, receipt sources, channels,
live loan legs, evidence, unresolved migration issues, latest gap reviews,
dismissals and the bounded gap worklist. Descriptor derivation uses the existing
R5 wallet implementation. Loading grants no ownership, network or tax authority.

Receipt checkpoint aliases now follow Python's last-value rule before merging
inventory high-water values, with the same minus-one floor. Receipt acquisition
failures run the existing accounting/tax admission and components stage in the
reference's exceptional-path order. Exact family/message/code comparisons cover
the original receipt error and earlier network, tax and component errors.
A conservative lexical filter skips scope parsing only when a 64-character hex
identity is impossible; escaped JSON and decoded legacy containers always take
the full parser. Empty inventory scripts allow raw-script/address fallback and
cannot create a source or trigger its deprecated-wallet validation. Fixtures
also separate distinct descriptor policies at the same horizon while checking
wallet-specific facts on shared current/retired cache hits. The filter and
derivation cache establish no new identity or ownership authority.

The unregistered `custody-load` example requires `test-harness`. Root mode opens
plaintext/SQLCipher through `crates/store`; SQL-copy mode uses a private local
pipe and includes pending caller writes. CI census/shadow jobs and the local
quality gate now explicitly build and enable the hook on disposable books.
Pure stage tests without a database connection have no loader comparison.
The existing observation-read budget remains enabled in census and deselected
only in shadow's deliberate two-build flow. See [testing](testing.md#rust-custody-comparisons)
and [privacy](privacy-and-security.md#custody-port-verification-data).

There are no new crates or versions in `Cargo.lock`. The Unix benchmark example
reuses the workspace's existing `nix` release for `getrusage`; the standard
library has no process CPU clock. No production runtime requirement changes.

## Byte gate and mutations

`crates/journals/scripts/generate_loader_vectors.py --check` regenerates
Python's default `json.dumps(..., ensure_ascii=True, allow_nan=True)` bytes.
The permissive historical fixture schema preserves column defaults, and
generation asserts nonempty ownership/receipt inputs to prevent vacuous gates.

| Gate | Result | Scope |
| --- | ---: | --- |
| Frozen generated SQLite fixtures | 37 exact matches | Structural columns/filters; legacy rates/network absence; config normalization; descriptor-cache isolation; current/retired policies; inventory fallbacks and receipt high-water cases |
| Frozen exceptional paths | 4 exact refusals | Exact receipt/network/tax/component family, message and code |
| Live generated database oracle | 44 passed | Includes both >50k worklist paths, graph identities with escaped JSON, and SQLCipher root/pending-write copies |
| Census and custody-shadow shards | 8/8 passed; 1,659 reported exact matches | Core-accounting, reports-contracts, daemon-cli and serial-integration under both modes |
| Generic 2k | 6,793,240 equal bytes | Complete generated book |
| Multi-wallet 2k | 9,238,672 equal bytes | Complete generated book, 171 paired pegs |
| Generic 20k | 68,023,010 equal bytes | Complete generated book |
| Multi-wallet 20k | 76,513,056 equal bytes | Complete generated book, 300 paired pegs |

| Mode | Shard | Passed | Skipped | Reported exact loader matches |
| --- | --- | ---: | ---: | ---: |
| Census | Core-accounting | 2,033 | 75 | 260 |
| Census | Reports-contracts | 2,519 | 10 | 377 |
| Census | Daemon-cli | 682 | 0 | 184 |
| Census | Serial-integration | 772 | 1,398 | 13 |
| Shadow | Core-accounting | 681 | 40 | 254 |
| Shadow | Reports-contracts | 1,919 | 10 | 374 |
| Shadow | Daemon-cli | 307 | 0 | 184 |
| Shadow | Serial-integration | 686 | 1,393 | 13 |

These are snapshot comparison counts, including repeated builds, rather than
distinct-book counts. All eight runs have zero loader or stage differences.
The observation-read budget runs in census; shadow retains the existing
exclusion of its two parameter cases. CI enables the hook on all nine shards;
local verification covers the four requested shard pairs. Parallel reruns use
CI's two-worker/loadscope policy; serial-integration stays serial. One test job
runs at a time. Skips include opt-in live regtest resources and unbuilt native
harness binaries; these results do not claim live regtest execution.

The recorder's omitted-match audit now corrupts only its recorded artifact;
the canonical database snapshot and production stages still compare. A refusal
harness regression also restores and asserts the incoming census/shadow mode,
so later books remain compared. Together, the focused modules pass **89 tests
and 11 reported loader matches per mode**. The separate source-overlap module passes
23 tests and three reported loader matches per mode. Post-load split-payout
injections declare their exact stage and exception; their snapshots still pass
the byte gate.

The loader mutation catalogue has **59 entries: 59 killed, zero survivors and
zero errors**. Its eight named baseline tests pass unmutated. The first
`JL-direct-active-count` recipe removed the exclusion filter and survived:
the bounded worklist independently rechecks the count and preserves the output.
The current recipe substitutes excluded observations for included ones, and
`native_loader_selects_bounded_worklist_from_active_count` kills it. The complete
current 59-entry catalogue was then rerun successfully. This remains a partial
catalogue, not exhaustive branch coverage. The inherited
2,285-entry custody mutation campaign is separate and has not been rerun for
this checkpoint.

## Timing and profile

`scripts/benchmark-custody-loader.py` uses disposable books from the committed
generator, generic seed 1 and multi-wallet seed 7, at 2k/20k. All four timing
books are plaintext; SQLCipher byte coverage is separate. Each book first
passes the complete byte gate. Measurements exclude open/BEGIN and v1
serialization; native total includes admission on the directly built snapshot.
Each book has five back-to-back Python/Rust pairs with alternating order.
CPU is process user+sys. Each implementation selects its best complete CPU
sample; the wall number comes from that same sample. Native load and admission
are never selected independently. The shared host had about ten other workers.

| Book | Python CPU / wall | Native load CPU / wall | Admission CPU / wall | Native total CPU / wall | 1-minute load |
| --- | ---: | ---: | ---: | ---: | ---: |
| Generic 2k | 0.119 / 0.120 s | 0.048 / 0.049 s | 0.005 / 0.005 s | 0.053 / 0.053 s | 15.18–15.37 |
| Multi-wallet 2k | 0.151 / 0.152 s | 0.068 / 0.068 s | 0.004 / 0.004 s | 0.072 / 0.073 s | 15.18 |
| Generic 20k | 1.189 / 1.197 s | 0.465 / 0.468 s | 0.041 / 0.041 s | **0.505 / 0.509 s** | 15.25–15.89 |
| Multi-wallet 20k | 1.446 / 1.457 s | 0.632 / 0.639 s | 0.055 / 0.056 s | 0.687 / 0.695 s | 15.58–16.69 |

The earlier checkpoint's generic-20k native total was 1.321 s wall. Its
[original records](r4-native-loader-bench.json) remain historical evidence;
they did not record process CPU, so the cross-run comparison is wall only.
The [new records](r4-native-loader-2-bench.json) retain all twenty pairs, generator
manifests, byte lengths/hashes, SQL floor repetitions and the release probe hash
`47791880351152ce1a70b9c64863c2ad1cc6e7b1dd3c07439b0eb63f2cbaca4a`.

The requested `pmp.py` profile took 400 samples on repeated timing-only generic
20k requests. Inclusive samples put 60.2% in `Reader::rows`, 15.5% in SQLite
stepping, 16.5% in collection construction, 7.5% in admission/validation, and
4.8% in receipt loading (28.5% before the lexical filter). These percentages
overlap. The profile includes book open and final drop, which the timer excludes;
29.8% of self frames were unresolved libc symbols, and the optimized build has
no source-line detail. This supports allocation/row-construction work as the
next investigation, not a precise allocator-only attribution.
At generic 20k, the transactions, network, rate and gap projections alone own
80,000 maps and 1,820,000 field keys (61/6/6/18 fields per row respectively).
The current map consumer requires an owned `String` for each key; reusing a
prepared layout cannot remove those allocations.

The probe also repeats the four dominant queries without decoding, five times
per book. Generic 20k's best CPU sum is 0.078 s (wall 0.078); multi-wallet 20k
is 0.098 (wall 0.098). This excludes ancillary component/ownership SQL and SQLite
shell text formatting, so it differs from the plan's historical two-query shell
floor. Repeating those original two queries in SQLite 3.54.0's shell five times
with output discarded gives **0.117 s CPU / 0.118 s wall** at generic 20k and
0.126 / 0.127 s at multi-wallet 20k, at one-minute load 22.41. Shell formatting
is included, as in the original floor. Every repetition is in the new records.

## Verification

Workspace formatting and offline workspace/all-target clippy with `-D warnings`
pass, including `kassiber-journals/test-harness`. The offline workspace suite
passes **1,501 tests, zero failed and zero ignored, across 115 test/doc-test
targets**. The locked Python environment was synchronized offline with Python
3.11; Cargo uses Rust 1.99.1 and `MATURIN_NO_INSTALL_RUST=1`. No new crate or
version was added. The required inventory update and the **57-test preflight
(62 subtests)** pass.

Only one test job ran at a time. Cargo used one test thread and four build jobs;
both `CARGO_TARGET_DIR` and `CARGO_BUILD_BUILD_DIR` pointed to the worktree's
`target` directory, including the mutation campaign. The initial workspace
attempt exposed an unreviewed benchmark-only `nix/resource` feature and generated
scratch symlinks outside the scanner's excluded roots. The feature was reviewed
and scratch moved under `target`; the scanner's exclusions were unchanged.
The full workspace suite then passed both policy checks and all other targets.

Mutation checkouts were removed automatically. After summaries were written,
the workspace/mutation `target/debug` tree was deleted. This worktree's earlier
7.3 GB debug tree in the machine's per-workspace Cargo archive was also removed.
Disposable books, profiles, logs and compact summaries remain under
`target/r4-loader-2`; no real book or user data root was used.

## Remaining acceptance work

1. Reach generic-20k 0.30 s. Direct construction removes the complete-snapshot
   conversion, but owning JSON/map records still costs 0.465 s CPU to load plus
   0.041 s admission. Finish the typed consumer boundary or demonstrate another
   byte-correct way to remove those allocations, then remeasure all four books.
2. Finish every loader branch's named-test mutation kills. A larger partial
   catalogue does not establish this acceptance requirement.
3. Finish lossless legacy `SqlValue` storage and refusal fidelity, including
   BLOB facts even where Python later refuses JSON serialization, nonfinite
   REALs and unusual storage/config/descriptor scalars across every read column.
4. Expand adversarial descriptor/source-index coverage beyond the current
   high-water, checkpoint alias, retired-policy and escaped-identity cases;
   repeat inherited generated/replay gates and a fresh correctness review
   before removing the test-only boundary.

## R4-13 integration contract

R4-13 must open through `crates/store`, hold `BEGIN IMMEDIATE` and its build
savepoint, run R4-10 repairs/pricing, then call the loader on that same connection
with the exact ordered profile and injected `as_of`. It must consume the eventual
typed `BookSnapshot` directly, roll back refusals/panics without refreshing
freshness markers, and commit only after successful persistence. CLI/daemon/AI
routes must not call this test-only prototype or adopt its serializer as a
production boundary. Accounting admission and watch-only/no-egress rules remain.
