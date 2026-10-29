# Rust read-path performance

[Round 2](rust-read-performance-round2.md) continues report and transaction-list
optimization from this checkpoint, with per-phase spans and schema proposals.

Local release measurements for `t3code/r5-read-perf`, based on `50f67616`.
No dependencies, indexes, migration markers or schema versions were added.
Python product code and accounting rules are unchanged.

## Method

Rust 1.99.1, offline Cargo, locked Python 3.11, normal release builds of
`kassiber`, and the supplied generic BTC books at 2,000 and 20,000 transactions.
Both books and the benchmark script were copied under this worktree's ignored
`target/read-perf/`; no original book was opened. `HOME` was not overridden by
benchmark commands. The normal ownership and unlock paths remained enabled.

Each cell is best of three subprocess calls including process start, captured
stdout/stderr and exit. Baseline Rust, candidate Rust and Python ran in rotating
order, without concurrent compilation or tests. All sixteen cases matched
**exit code, stdout bytes and stderr bytes** across all three engines.
Machine load differs from the original user measurements; the saved baseline
binary was rebuilt from `50f67616` in this worktree for comparison.

## End-to-end results

Milliseconds; `before → after` refers to Rust. Python is this checkout's
reference on the same copied book.

| Command | 2,000 Rust | 20,000 Rust | 2,000 Python | 20,000 Python |
| --- | ---: | ---: | ---: | ---: |
| `status` | 103.7 → 19.0 | 145.6 → 36.9 | 426.5 | 480.0 |
| `wallets list` | 104.8 → 18.4 | 147.1 → 39.2 | 422.3 | 487.6 |
| `transactions list --limit 100` | 112.6 → 27.9 | 218.4 → 101.2 | 437.8 | 530.5 |
| `reports summary` | 135.5 → 38.0 | 561.1 → 263.3 | 502.3 | 1315.2 |
| `reports balance-sheet` | 123.3 → 26.2 | 410.8 → 145.2 | 459.8 | 835.0 |
| `reports capital-gains` | 132.9 → 35.1 | 514.5 → 230.1 | 487.2 | 1126.7 |
| `reports tax-summary` | 127.5 → 27.2 | 429.9 → 151.9 | 471.3 | 846.8 |
| `reports balance-history` | 138.2 → 37.2 | 533.8 → 253.0 | 485.5 | 975.2 |

## Release startup spans

Temporary `Instant` spans were added to local builds of both revisions and
removed before verification. They print only fixed labels and elapsed times
under an explicit scratch profiling flag. No product diagnostics or disk logging
were added. Neither `strace`, `ltrace` nor a sampling profiler was available.

The following spans come from the fastest of three instrumented `status`
processes. Book open includes ordered compatibility and admission; admission
includes structure. **Nested rows must not be added together.**

| Stage (ms) | 2,000 before | 2,000 after | 20,000 before | 20,000 after |
| --- | ---: | ---: | ---: | ---: |
| Argument parsing | 0.072 | 0.080 | 0.074 | 0.079 |
| Unlock-mode check | 0.496 | 0.590 | 0.486 | 0.497 |
| Path/config bootstrap | 0.641 | 0.786 | 0.662 | 0.678 |
| Book open, inclusive | 84.283 | 11.986 | 122.362 | 29.725 |
| ↳ Ordered compatibility | 22.883 | 6.116 | 43.434 | 15.835 |
| ↳ Schema/semantic admission | 56.708 | 1.428 | 73.951 | 9.272 |
| ↳↳ Reconstructed structural comparison | 54.464 | 0.000 | 54.024 | 0.000 |
| Status queries | 0.381 | 0.333 | 1.205 | 1.590 |
| Output rendering | 0.025 | 0.013 | 0.024 | 0.024 |
| Book close | 15.496 | 0.620 | 16.594 | 2.737 |
| Main, inclusive | 102.030 | 14.898 | 142.041 | 35.939 |
| Whole subprocess | 107.274 | 18.550 | 147.360 | 40.415 |

The release binary is about 53 MiB. A corresponding `--version` process takes
4.9–6.1 ms including 2.6–2.9 ms in argument parsing; process loading is not the
large startup cost. The difference between whole-process and main spans also
includes process teardown and capture overhead, not just dynamic linking.

## Changes

- **Schema comparison:** exact `sqlite_master` definitions prove the current
  structure without creating another SQLite database and introspecting both.
  Historical ALTER layouts still use the complete normalized structural gate.
  Audits and every semantic probe still run in the same consistent snapshot.
- **Compatibility work:** ordered column ranges load each table's columns once.
  Existing replacement indexes, projection view and immutability triggers are
  compared before DDL. Matching definitions avoid schema-cookie changes and
  close-time WAL checkpoints. Obsolete, missing and changed definitions retain
  the original repairs and transaction boundaries.
- **Liquid checks:** identical asset predicates first run over a materialized
  distinct-asset list using existing covering indexes. BTC-only books prove the
  Liquid worklist empty without two wide observation scans. Real Liquid rows
  still follow the original migration and validation paths.
- **Network admission:** the shared evidence interpreter checks every relevant
  observation and reference, but accounting admission does not construct the
  review-only digest, transaction clones or presentation payload. A conservative
  negative lexical filter proves when JSON contains no network keys; escaped or
  potentially relevant keys always take the Python-compatible parser. Within a
  single wallet/read, those neutral observations reuse evidence by asset.
  Conflicts, binding comparisons and empty-wallet evidence remain checked.
- **Report projection:** holdings stream quote candidates instead of retaining
  an entire unused transaction projection. Late SQLite/codec errors retain
  precedence over earlier interpretation errors. Summary reads the same ordered
  capital-gains rows, validates identity and sums the same floats, without
  constructing unused quantity/reference presentation. SQL row buffers reserve
  their column count.

At 20,000 transactions the instrumented summary network-admission span falls
from 233.5 to 74.6 ms, and summary construction from 184.4 to 142.0 ms. The
balance-sheet holdings span falls from 26.6 to 23.3 ms while avoiding retention
of the complete candidate table.

## Verification

- `cargo fmt --all -- --check`: passed.
- Offline workspace/all-target/all-feature clippy with `-D warnings`: passed.
- Offline workspace/all-feature tests: **1,192 passed**, none failed or ignored
  across 92 test binaries and doctest groups. Local socket fixtures required
  execution outside the socket-restricted sandbox.
- Shared-caller suites through the hybrid candidate: **507 passed and 62 subtests
  passed**. These cover wallet setup/descriptors, imports, review regressions,
  CLI smoke, silent payments, rates CSV validation, book-network boundaries,
  transaction listing, report formats/blockers, year handoff and filed chains.
- Isolated native report parity: **19 passed**, covering generic FIFO/HIFO and
  Austrian moving-average books, JSON/text/table/CSV bytes, daemon reads,
  readiness refusals, cached quotes and atomic file output. Per-test routing
  logs assert native report delivery. An aggregate routing log in the broader
  run was unsuitable for these per-test assertions, so this suite was rerun
  with isolated logs.
- `classify_tests update`: passed, with no inventory changes.
- Preflight shard: **57 passed and 57 subtests passed**.
- Final normal release rebuild: all sixteen benchmark cases match the saved
  measured candidate's exit/stdout/stderr bytes; no profiling output remains.
- Manual second review and staged/unstaged whitespace checks: passed.

The broader invocation of every `tests/test_rust_*.py` module, CLI-runner tests
and `tests/rust_port` completed with **2,245 passed, 31 skipped, 341 subtests
passed and 14 failed**. Twelve failures were report routing assertions polluted
by earlier unported export calls in the aggregate log; all report byte
comparisons reached before those assertions passed, and the complete isolated
19-test rerun passed. The 31 skips are optional keyring scenarios. This broad
invocation is not reported as fully green.

The remaining two failures are encrypted desktop-backup cases that fail before
candidate execution: reference Python rejects its own preview with
`stale_backup_preview`. Both were reproduced in isolated runs with diagnostics
confirming `reference=True`: `test_apply_generation_guards_and_safe_restore_match_raw_jsonl[none]`
and `test_other_owner_refusal_retains_preview_until_safe_apply`. Fixture setup
and the failing operation execute Python; Rust has not run when either failure
occurs. Python product/test sources are unchanged from the baseline. These
failures are not waived as successful parity.

The final audit of 34 surviving routing logs records **866 calls** across the
12 touched CLI read commands, **all Rust, zero Python fallback**: status 24,
wallet list 2, transaction list 233; summary 79, balance sheet 54, portfolio 59,
capital gains 67, tax summary 54, journal entries 63, balance history 125,
Austrian E1kv 59 and Austrian tax summary 47. Counts include refusal cases and
repeat runs, not just successful benchmark calls. Unported setup, legacy export
and journal-processing calls retain their intended Python routing and are
excluded from that read-command total. Daemon byte comparisons are covered by
the native report, setup, transaction and shell suites. There is no full
quality-gate or push-readiness claim.

## Limits and deliberately retained work

The all-reports-in-tens-of-milliseconds target is **not reached** at 20,000
transactions: reports now take roughly 145–263 ms on this machine. Network
scope still examines current evidence, admission still checks pending semantic
work, and summary/history still process ordered rows with exact arithmetic and
float behavior. Capital gains emits 6.34 MB in this benchmark. Historical
schemas can still take the slower normalized comparison.

No new index was installed. Further indexes require the shared Python/Rust
schema and migration procedure, so no such change was attempted. In particular,
there is no persisted “already checked” shortcut, no skipped schema/ownership/
unlock/readiness gate, no cross-call admission cache, and no guessed tax or
custody state. The benchmark did not exercise encrypted-book KDF performance;
SQLCipher interoperability and refusal paths are covered separately by tests.

Scratch scripts, saved binaries, copied books, span records and raw benchmark
results remain under `target/read-perf/`; they are not committed or published.
