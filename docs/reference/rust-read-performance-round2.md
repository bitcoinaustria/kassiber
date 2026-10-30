# Rust read-path performance, round 2

Local work on `t3code/r5-read-perf-2`, based on round 1 (`3d898f9a`).
See [round 1](rust-read-performance.md) for the preceding startup work.
Python product code, dependencies, schema objects and migration markers are
unchanged. No changes were pushed or published.

## Method

Rust 1.99.1 through mise, offline Cargo, `MATURIN_NO_INSTALL_RUST=1`, and
`uv sync --locked --python 3.11`. Both supplied books and the supplied benchmark
script were copied under `target/read-perf-2/`. Adaptations compare exit status,
stdout bytes **and stderr bytes**, retain a base executable, and use the best of
three subprocess runs. The final comparison uses ordinary release executables
without the test harness or profiling instrumentation. `HOME` is never overridden
by these commands. Ownership, unlock, compatibility and readiness remain active.

The phase measurements use temporary `Instant` instrumentation, removed before
final verification. Counters are per-thread, RAM-only, and print fixed labels
only when the explicit scratch profiling flag is set. They separate SQLite
prepare/iteration, conversion to Python-compatible row values, decimal parsing,
arithmetic and formatting, and the remaining aggregation/presentation work.
Serialization measures the final JSON encoder. Report construction starts at the
service entry, so book opening and process startup are outside these spans.
Instrumented measurements include timer overhead and must not be substituted
for normal release subprocess timings. Scratch code, copied books, source
snapshots, binaries, routing logs and raw results remain ignored under `target/`.

## End-to-end measurements

Milliseconds, base → candidate; best of three complete subprocess calls.
All sixteen cases match Python and the base in exit status, stdout and stderr.

| Command | 2,000 Rust | 20,000 Rust | 2,000 Python | 20,000 Python |
| --- | ---: | ---: | ---: | ---: |
| `status` | 20.6 → 18.2 | 38.0 → 39.5 | 388.5 | 439.7 |
| `wallets list` | 17.1 → 17.8 | 36.4 → 37.0 | 376.3 | 437.8 |
| `transactions list --limit 100` | 24.7 → 24.4 | 97.2 → 71.8 | 386.8 | 530.8 |
| `reports summary` | 37.1 → 32.9 | 266.6 → 214.5 | 455.3 | 1302.7 |
| `reports balance-sheet` | 27.8 → 22.2 | 133.0 → 97.1 | 439.1 | 806.0 |
| `reports capital-gains` | 35.0 → 32.3 | 220.7 → 187.7 | 457.8 | 1073.5 |
| `reports tax-summary` | 28.0 → 24.1 | 149.5 → 110.6 | 442.2 | 791.7 |
| `reports balance-history` | 37.8 → 30.4 | 244.1 → 171.1 | 444.0 | 948.9 |

Untargeted status/wallet results show millisecond-scale startup variation. At 20,000
transactions, listing improves **26%**, summary **20%**, balance sheet **27%**,
capital gains **15%**, tax summary **26%**, and history **30%**.

## Per-phase measurements at 20,000 transactions

Milliseconds, before → after; one fastest construction run per command, chosen
from three instrumented calls. The five phase columns are disjoint; construction
is SQL + decoding + Decimal + aggregation. Serialization follows construction.
“SQL” includes statement preparation, stepping and driver iteration; aggregation
also includes timestamps, evidence interpretation and presentation allocation.

| Command | SQL/driver | Row decoding | Decimal/BigInt | Aggregation/presentation | Serialization |
| --- | ---: | ---: | ---: | ---: | ---: |
| `transactions list --limit 100` | 51.68 → 31.74 | 0.34 → 0.35 | 0.00 → 0.00 | 0.82 → 0.85 | 0.35 → 0.36 |
| `reports summary` | 135.34 → 133.00 | 41.93 → 19.57 | 0.04 → 0.04 | 40.48 → 11.26 | 0.03 → 0.02 |
| `reports balance-sheet` | 38.16 → 36.43 | 33.35 → 15.18 | 0.03 → 0.02 | 29.43 → 10.16 | 0.01 → 0.01 |
| `reports capital-gains` | 54.03 → 54.37 | 32.83 → 19.74 | 7.89 → 1.02 | 42.89 → 27.09 | 22.83 → 22.99 |
| `reports tax-summary` | 55.84 → 55.43 | 23.85 → 7.09 | 0.06 → 0.05 | 25.23 → 7.44 | 0.04 → 0.04 |
| `reports balance-history` | 53.49 → 54.87 | 41.43 → 19.61 | 35.27 → 13.03 | 79.09 → 51.05 | 0.13 → 0.10 |

The Decimal/BigInt column combines instrumented decimal construction, parsing,
arithmetic and decimal formatting; SQLite-integer-to-JSON BigInt decoding is in
row decoding. Within history it falls from **35.27 to 13.03 ms**: arithmetic/
construction **21.66 → 4.66**, parsing **13.58 → 8.32**, and formatting about
**0.03 → 0.03 ms**. (The exact subspan values are retained in the scratch JSON.)

Network admission is nested within these columns, **not an extra additive phase**:
summary **74.28 → 41.25 ms**, balance sheet **72.68 → 39.06**, capital gains
**74.59 → 39.18**, tax summary **74.21 → 40.26**, history **74.69 → 39.74**.
The service construction totals are respectively **217.79 → 163.87**,
**100.97 → 61.80**, **137.65 → 102.22**, **104.97 → 70.01** and
**209.28 → 138.56 ms**. These exclude the roughly 30–40 ms open/startup path.

## Implementation

- Network admission streams the same ordered six-column observation projection
  into a reusable row dictionary. Every selected SQLite value still passes the
  same codec validation. Per-call wallet lookups avoid a wallet scan per row;
  neutral evidence is interpreted once per wallet and asset and merged once
  into admission's sets. Empty-wallet configuration, conflicting assets,
  escaped network keys and reference evidence retain their existing gates.
  The review inventory still constructs its complete evidence and digest.
- Streaming SQL visitors reuse column keys and unchanged text values. A report
  interpretation error is held until the entire SQL projection has decoded,
  preserving late SQLite/codec error precedence.
- Summary folds the complete ordered capital-gains projection into three float
  accumulators without retaining its rows. It validates database identity for
  any returned row before returning the report, including wallet scopes whose
  matching subset is empty. Each accumulator retains Python's addition order.
- History converts SQL rows directly to typed events and quotes without keeping
  intermediate dictionaries. Integer msat scale directly by `10^-11`; SQLite
  integers have at most 19 significant digits and fit the precision-32 context
  exactly. Legacy string/REAL conversions and their errors keep the old path.
  Null/empty and SQLite integer decimal inputs avoid text formatting/parsing.
- Transaction listing materializes only the ordered page's SQLite rowids before
  fetching its wide pricing and description projection. This avoids filling
  SQLite's top-N sorter with wide rows for the entire book. Rowids preserve even
  SQLite's permitted null text primary keys. Filters, count, sort tie-breaks,
  bound cursors and tag ordering are unchanged.

No ordered Decimal or float sum was replaced with SQLite `SUM`: changed addition
order, float coercion, integer overflow and precision-32 rounding would require
proof beyond the generic fixture. Existing exact SQL aggregates remain SQL.
Admission still reads current evidence; no cross-call validity cache is added.

## Schema proposal — measured, not implemented

The owner subsequently approved only the page index; its implementation and
new measurements are tracked in [transaction page index](transaction-page-index.md).
The flow and metrics indexes below remain unimplemented proposals.

Measurements below use rusqlite's bundled SQLite **3.51.3**, the same library as
the native service, on another disposable copy of the 20,000-row book. Each
query is prepared, bound, stepped and decoded; results are compared exactly
before/after every stage. Values are best of seven warm runs. Indexes are added
cumulatively in the listed order; these are SQL microbenchmarks, not claimed
end-to-end CLI timings.

| Query (ms) | No new index | Page index | + flow index | + metrics index |
| --- | ---: | ---: | ---: | ---: |
| Listing count | 24.78 | 3.32 | 3.25 | 3.23 |
| Listing ordered 101 rowids | 16.15 | 0.030 | 0.030 | 0.030 |
| Summary metrics | 34.69 | 15.46 | 34.04 | 10.25 |
| Summary asset flow | 20.15 | 19.41 | 9.06 | 9.20 |
| Summary wallet flow | 23.90 | 22.91 | 16.75 | 16.96 |

Proposed additive DDL (names remain proposals):

```sql
CREATE INDEX proposed_transactions_profile_page
ON transactions(profile_id, occurred_at, created_at, id, wallet_id);

CREATE INDEX proposed_transactions_profile_flow
ON transactions(profile_id, excluded, asset, wallet_id,
                direction, amount, fee, occurred_at);

CREATE INDEX proposed_transactions_profile_metrics
ON transactions(profile_id, excluded, direction, asset, occurred_at,
                fiat_rate, fiat_value, note);
```

The page index is the strongest independent proposal: SQLite switches from a
profile scan plus temporary sort to an ordered covering scan that stops at 101
rows. Its `wallet_id` also makes the exact count's wallet join covering. The
count is still O(N), but no longer fetches each wide transaction row. It uses
**3.14 MiB** on this fixture. The flow and metrics indexes use **2.40 MiB** and
**1.80 MiB** respectively, measured by `dbstat` index page totals. Creation took
roughly 50–90 ms per index on the scratch copy; large/encrypted books and slower
storage require their own measurements.

The flow index alone causes the planner to regress summary metrics from 15.46
to 34.04 ms; the metrics index then produces a covering metrics plan. Consider
the two report indexes together, rather than accepting the flow index by
itself. None makes report admission sub-linear: network evidence is still read.
The figures do not justify promising all reports in tens of milliseconds.

Risks include larger encrypted/plaintext books, write amplification on imports,
pricing and note edits, longer first opens, temporary disk requirements, and
planner regressions for many-profile, wallet-filtered, unusual-asset and
historical books. The metrics index duplicates note text inside the same book;
large notes may make its cost disproportionate. No index should be approved
solely from this one-profile generic fixture.

Any accepted index must be added to **both** Python's fresh schema and ordered
compatibility helper and Rust's matching ordered pass. Rebuild the generated
creation transcript and current-schema structural definitions through their
normal generators, update the schema drift fixtures, and preserve the exact
shared definitions. Use a transaction and idempotent definition checks; do not
recreate an unchanged index on every open or change the schema cookie needlessly.
These nonunique indexes add no economic authority, version counter or backfill
of accounting facts. Do not use `ANALYZE` or global planner hints as an implicit
migration.

Acceptance must cover fresh books, current books without the indexes, both
historical compatibility fixtures and SQLCipher copies. Python-first and
Rust-first upgrades must finish on the first open, preserve a complete logical
row dump, and leave the following Python and Rust opens schema-idempotent.
Exercise interruption/rollback during index creation and busy/full-disk errors.
Existing Python readers can ignore the indexes, but older Rust structural gates
must be tested explicitly; do not assume an extra index is accepted. If they
refuse it, require the compatible Rust release before creating the index rather
than weakening admission. Retain existing indexes until query-plan and write
benchmarks justify their removal. See [database compatibility](database-compatibility.md).

A persisted network-admission aggregate is **not proposed for implementation**:
its authority would depend on all transaction, wallet, observer/reference and
migration writes, not merely `journal_input_version`. A naive last-checked flag
would accept stale evidence. Similarly, a summary cache would need exact
Python-order float/Decimal semantics, tax-override handling and atomic
invalidation across both writers. This round supplies no measured validated
aggregate design.

## Verification

- Offline workspace/all-feature tests: **1,196 passed**, zero failed or ignored,
  across 92 test binaries/doctest groups, including the null-primary-key page
  regression. Disposable loopback fixtures require execution outside the socket
  sandbox; the first restricted run stopped on those fixture denials.
- Workspace/all-target/all-feature clippy with `-D warnings`: passed.
  `cargo fmt --all -- --check`: passed.
- `classify_tests update`: passed, with no inventory changes. Final preflight:
  **57 passed and 57 subtests passed**.
- Shared-caller selection: **236 tests** covered transaction pages/services/
  desktop, setup desktop, CLI smoke, report formatting/cross-checks/blockers,
  year handoff, filed chains and book-network boundaries/read snapshots.
  The initial restricted run had 230 passes, 69 subtests and six failures.
  Five captured reference-daemon failures attempted to chmod Python's standard
  owner-lock directory on a read-only mount. The affected modules plus the
  AI-provider smoke case passed outside that sandbox: **13 passed, 57 subtests**.
  The initial AI smoke failure's precise cause was not instrumented; it passed
  that rerun. The final listing-only run also passed **5 tests and 57 subtests**.
- Native report parity: **19 passed** on the final hybrid candidate. Covers generic
  FIFO/HIFO and Austrian moving-average books, JSON/text/table/CSV bytes,
  atomic file output, cached quotes, readiness/error precedence and daemon
  holdings. Per-test routing logs assert native report execution.
- All three relevant mutation catalogs validate. **Seven selected mutations
  killed**: the two new wallet/asset cache scopes; exact BTC scaling; and four
  adjusted anchors for quote selection, summary wallet scoping, ordered
  realized sums and history's fiat-value amount conversion. Other existing
  mutation entries were validated, not all executed.
- All sixteen final ordinary-release benchmark cases match base and Python in
  exit code, stdout and stderr. Profiling sources and scratch tests are removed;
  product code contains no profiling flag, span output or diagnostic writes.
- Routing audit: **1,100 calls across 39 retained logs**, all Rust and
  **zero fallback** for the nine live report commands and transaction listing.
  Counts include refusals and repeated runs. Unported journal-processing/setup
  or export operations retain their intended Python routing and are excluded;
  daemon byte comparisons are covered by native report/listing/service tests.
- Manual second review and staged/unstaged whitespace checks passed. Local-only
  work: no push, PR, schema migration or full frontend quality-gate claim.

## Desktop backup finding

The requested once-only base rerun **passed both cases**, including reference
Python followed by the Rust candidate:

```sh
KASSIBER_RUST_BIN="$PWD/target/read-perf-2/base" .venv/bin/python -m pytest -q \
  'tests/test_rust_backup_desktop.py::test_apply_generation_guards_and_safe_restore_match_raw_jsonl[none]' \
  tests/test_rust_backup_desktop.py::test_other_owner_refusal_retains_preview_until_safe_apply
```

Result: **2 passed in 16.62 s** on unmodified `3d898f9a`. Therefore this run does
not establish a deterministic Python bug, a timing cause or an environment
cause for round 1's `stale_backup_preview`. No Python fix or failure waiver was
introduced. The command above reproduces the attempted check; the reported
failure did not reproduce.

## Remaining limits

The all-reports-in-tens-of-milliseconds goal remains **unmet** at 20,000
transactions. Summary still takes about 214 ms, capital gains 188 ms and history
171 ms on this machine. A capital-gains response contains 6.34 MB, so encoding,
transport capture and destruction scale with emitted rows even if selection
were constant-time. Network admission must still examine current evidence.

The default listing still needs an exact filtered count and a book-wide narrow
sort without a matching index. Wide projection is now bounded by the page size,
but the whole operation is not sub-linear. The measured page-index proposal
addresses ordered selection; its exact count remains linear over a small
covering index. No persisted aggregate, skipped blocker/readiness check,
background egress or unreviewed migration was introduced.
